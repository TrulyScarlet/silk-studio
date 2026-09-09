use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::Serialize;
use std::convert::TryFrom;
use std::io::{Read, Seek, Write};

use crate::mp4box::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Av1CBox {
    pub seq_profile: u8,
    pub seq_level_idx_0: u8,
    pub seq_tier_0: bool,
    pub high_bitdepth: bool,
    pub twelve_bit: bool,
    pub monochrome: bool,
    pub chroma_subsampling_x: bool,
    pub chroma_subsampling_y: bool,
    pub chroma_sample_position: u8,
    pub initial_presentation_delay_minus_one: Option<u8>,
    pub config_obus: Vec<u8>,
}

impl Default for Av1CBox {
    fn default() -> Self {
        Self {
            seq_profile: 0,
            seq_level_idx_0: 0,
            seq_tier_0: false,
            high_bitdepth: false,
            twelve_bit: false,
            monochrome: false,
            chroma_subsampling_x: true,
            chroma_subsampling_y: true,
            chroma_sample_position: 0,
            initial_presentation_delay_minus_one: None,
            config_obus: Vec::new(),
        }
    }
}

impl TryFrom<&Av1Config> for Av1CBox {
    type Error = Error;

    fn try_from(config: &Av1Config) -> Result<Self> {
        config.validate()?;
        Ok(Av1CBox {
            seq_profile: config.seq_profile,
            seq_level_idx_0: config.seq_level_idx_0,
            seq_tier_0: config.seq_tier_0,
            high_bitdepth: config.high_bitdepth,
            twelve_bit: config.twelve_bit,
            monochrome: config.monochrome,
            chroma_subsampling_x: config.chroma_subsampling_x,
            chroma_subsampling_y: config.chroma_subsampling_y,
            chroma_sample_position: config.chroma_sample_position,
            initial_presentation_delay_minus_one: config.initial_presentation_delay_minus_one,
            config_obus: config.config_obus.clone(),
        })
    }
}

impl TryFrom<Av1Config> for Av1CBox {
    type Error = Error;

    fn try_from(config: Av1Config) -> Result<Self> {
        Self::try_from(&config)
    }
}

impl Av1CBox {
    pub fn new(config: &Av1Config) -> Result<Self> {
        Self::try_from(config)
    }
}

impl Mp4Box for Av1CBox {
    fn box_type(&self) -> BoxType {
        BoxType::Av1CBox
    }

    fn box_size(&self) -> u64 {
        HEADER_SIZE + 4 + self.config_obus.len() as u64
    }

    fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(&self).unwrap())
    }

    fn summary(&self) -> Result<String> {
        let s = format!(
            "seq_profile={} seq_level_idx_0={} config_obus_len={}",
            self.seq_profile,
            self.seq_level_idx_0,
            self.config_obus.len()
        );
        Ok(s)
    }
}

impl<R: Read + Seek> ReadBox<&mut R> for Av1CBox {
    fn read_box(reader: &mut R, size: u64) -> Result<Self> {
        let start = box_start(reader)?;

        if size < HEADER_SIZE + 4 {
            return Err(Error::InvalidData("av1C box size too small"));
        }

        let byte0 = reader.read_u8()?;
        let marker = (byte0 >> 7) & 0x01;
        let version = byte0 & 0x7F;
        if marker != 1 || version != 1 {
            return Err(Error::InvalidData("invalid av1C marker or version"));
        }

        let byte1 = reader.read_u8()?;
        let seq_profile = (byte1 >> 5) & 0x07;
        let seq_level_idx_0 = byte1 & 0x1F;

        let byte2 = reader.read_u8()?;
        let seq_tier_0 = (byte2 & 0x80) != 0;
        let high_bitdepth = (byte2 & 0x40) != 0;
        let twelve_bit = (byte2 & 0x20) != 0;
        let monochrome = (byte2 & 0x10) != 0;
        let chroma_subsampling_x = (byte2 & 0x08) != 0;
        let chroma_subsampling_y = (byte2 & 0x04) != 0;
        let chroma_sample_position = byte2 & 0x03;

        let byte3 = reader.read_u8()?;
        let initial_presentation_delay_present = (byte3 & 0x10) != 0;
        let initial_presentation_delay_minus_one = if initial_presentation_delay_present {
            Some(byte3 & 0x0F)
        } else {
            None
        };

        let config_obus_len = (size - HEADER_SIZE - 4) as usize;
        let mut config_obus = vec![0u8; config_obus_len];
        reader.read_exact(&mut config_obus)?;

        skip_bytes_to(reader, start + size)?;

        Ok(Av1CBox {
            seq_profile,
            seq_level_idx_0,
            seq_tier_0,
            high_bitdepth,
            twelve_bit,
            monochrome,
            chroma_subsampling_x,
            chroma_subsampling_y,
            chroma_sample_position,
            initial_presentation_delay_minus_one,
            config_obus,
        })
    }
}

impl<W: Write> WriteBox<&mut W> for Av1CBox {
    fn write_box(&self, writer: &mut W) -> Result<u64> {
        let size = self.box_size();
        BoxHeader::new(self.box_type(), size).write(writer)?;

        // Byte 0: marker=1 (bit 7), version=1 (bits 6..0) -> 0x81
        writer.write_u8(0x81)?;

        // Byte 1: seq_profile (3 bits), seq_level_idx_0 (5 bits)
        let byte1 = ((self.seq_profile & 0x07) << 5) | (self.seq_level_idx_0 & 0x1F);
        writer.write_u8(byte1)?;

        // Byte 2: seq_tier_0(1), high_bitdepth(1), twelve_bit(1), monochrome(1),
        //         chroma_subsampling_x(1), chroma_subsampling_y(1), chroma_sample_position(2)
        let byte2 = (if self.seq_tier_0 { 0x80 } else { 0 })
            | (if self.high_bitdepth { 0x40 } else { 0 })
            | (if self.twelve_bit { 0x20 } else { 0 })
            | (if self.monochrome { 0x10 } else { 0 })
            | (if self.chroma_subsampling_x { 0x08 } else { 0 })
            | (if self.chroma_subsampling_y { 0x04 } else { 0 })
            | (self.chroma_sample_position & 0x03);
        writer.write_u8(byte2)?;

        // Byte 3: reserved(3)=0, initial_presentation_delay_present(1), initial_presentation_delay_minus_one(4)
        let byte3 = if let Some(delay) = self.initial_presentation_delay_minus_one {
            0x10 | (delay & 0x0F)
        } else {
            0x00
        };
        writer.write_u8(byte3)?;

        // configOBUs
        writer.write_all(&self.config_obus)?;

        Ok(size)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Av01Box {
    pub data_reference_index: u16,
    pub width: u16,
    pub height: u16,

    #[serde(with = "value_u32")]
    pub horizresolution: FixedPointU16,

    #[serde(with = "value_u32")]
    pub vertresolution: FixedPointU16,
    pub frame_count: u16,
    pub depth: u16,
    pub av1c: Av1CBox,
}

impl Default for Av01Box {
    fn default() -> Self {
        Av01Box {
            data_reference_index: 0,
            width: 0,
            height: 0,
            horizresolution: FixedPointU16::new(0x48),
            vertresolution: FixedPointU16::new(0x48),
            frame_count: 1,
            depth: 0x0018,
            av1c: Av1CBox::default(),
        }
    }
}

impl Av01Box {
    pub fn new(config: &Av1Config) -> Result<Self> {
        let av1c = Av1CBox::try_from(config)?;
        Ok(Av01Box {
            data_reference_index: 1,
            width: config.width,
            height: config.height,
            horizresolution: FixedPointU16::new(0x48),
            vertresolution: FixedPointU16::new(0x48),
            frame_count: 1,
            depth: 0x0018,
            av1c,
        })
    }

    pub fn get_type(&self) -> BoxType {
        BoxType::Av01Box
    }

    pub fn get_size(&self) -> u64 {
        HEADER_SIZE + 8 + 70 + self.av1c.box_size()
    }
}

impl TryFrom<&Av1Config> for Av01Box {
    type Error = Error;

    fn try_from(config: &Av1Config) -> Result<Self> {
        Self::new(config)
    }
}

impl TryFrom<Av1Config> for Av01Box {
    type Error = Error;

    fn try_from(config: Av1Config) -> Result<Self> {
        Self::new(&config)
    }
}

impl Mp4Box for Av01Box {
    fn box_type(&self) -> BoxType {
        self.get_type()
    }

    fn box_size(&self) -> u64 {
        self.get_size()
    }

    fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(&self).unwrap())
    }

    fn summary(&self) -> Result<String> {
        let s = format!(
            "data_reference_index={} width={} height={} frame_count={}",
            self.data_reference_index, self.width, self.height, self.frame_count
        );
        Ok(s)
    }
}

impl<R: Read + Seek> ReadBox<&mut R> for Av01Box {
    fn read_box(reader: &mut R, size: u64) -> Result<Self> {
        let start = box_start(reader)?;

        reader.read_u32::<BigEndian>()?; // reserved
        reader.read_u16::<BigEndian>()?; // reserved
        let data_reference_index = reader.read_u16::<BigEndian>()?;

        reader.read_u32::<BigEndian>()?; // pre-defined, reserved
        reader.read_u64::<BigEndian>()?; // pre-defined
        reader.read_u32::<BigEndian>()?; // pre-defined
        let width = reader.read_u16::<BigEndian>()?;
        let height = reader.read_u16::<BigEndian>()?;
        let horizresolution = FixedPointU16::new_raw(reader.read_u32::<BigEndian>()?);
        let vertresolution = FixedPointU16::new_raw(reader.read_u32::<BigEndian>()?);
        reader.read_u32::<BigEndian>()?; // reserved
        let frame_count = reader.read_u16::<BigEndian>()?;
        skip_bytes(reader, 32)?; // compressorname
        let depth = reader.read_u16::<BigEndian>()?;
        reader.read_i16::<BigEndian>()?; // pre-defined

        let header = BoxHeader::read(reader)?;
        let BoxHeader { name, size: s } = header;
        if s > size {
            return Err(Error::InvalidData(
                "av01 box contains a box with a larger size than it",
            ));
        }
        if name == BoxType::Av1CBox {
            let av1c = Av1CBox::read_box(reader, s)?;

            skip_bytes_to(reader, start + size)?;

            Ok(Av01Box {
                data_reference_index,
                width,
                height,
                horizresolution,
                vertresolution,
                frame_count,
                depth,
                av1c,
            })
        } else {
            Err(Error::InvalidData("av1C not found"))
        }
    }
}

impl<W: Write> WriteBox<&mut W> for Av01Box {
    fn write_box(&self, writer: &mut W) -> Result<u64> {
        let size = self.box_size();
        BoxHeader::new(self.box_type(), size).write(writer)?;

        writer.write_u32::<BigEndian>(0)?; // reserved
        writer.write_u16::<BigEndian>(0)?; // reserved
        writer.write_u16::<BigEndian>(self.data_reference_index)?;

        writer.write_u32::<BigEndian>(0)?; // pre-defined, reserved
        writer.write_u64::<BigEndian>(0)?; // pre-defined
        writer.write_u32::<BigEndian>(0)?; // pre-defined
        writer.write_u16::<BigEndian>(self.width)?;
        writer.write_u16::<BigEndian>(self.height)?;
        writer.write_u32::<BigEndian>(self.horizresolution.raw_value())?;
        writer.write_u32::<BigEndian>(self.vertresolution.raw_value())?;
        writer.write_u32::<BigEndian>(0)?; // reserved
        writer.write_u16::<BigEndian>(self.frame_count)?;
        // skip compressorname
        write_zeros(writer, 32)?;
        writer.write_u16::<BigEndian>(self.depth)?;
        writer.write_i16::<BigEndian>(-1)?; // pre-defined

        self.av1c.write_box(writer)?;

        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4box::BoxHeader;
    use std::io::Cursor;

    fn sample_av1_config() -> Av1Config {
        Av1Config {
            width: 1920,
            height: 1080,
            seq_profile: 0,
            seq_level_idx_0: 8,
            seq_tier_0: false,
            high_bitdepth: false,
            twelve_bit: false,
            monochrome: false,
            chroma_subsampling_x: true,
            chroma_subsampling_y: true,
            chroma_sample_position: 0,
            initial_presentation_delay_minus_one: Some(2),
            // Valid Sequence Header OBU: type 1, has_size_field=1 -> header byte 0x0A, size 4, payload 4 bytes
            config_obus: vec![0x0A, 0x04, 0x20, 0x00, 0x00, 0x00],
        }
    }

    #[test]
    fn test_av01_and_av1c_exact_bytes_and_roundtrip() {
        let config = sample_av1_config();
        let av01 = Av01Box::new(&config).unwrap();

        // Check box types and FourCC
        assert_eq!(av01.box_type(), BoxType::Av01Box);
        assert_eq!(av01.av1c.box_type(), BoxType::Av1CBox);
        let fcc = FourCC::from(av01.box_type());
        assert_eq!(&fcc.value, b"av01");
        let fcc_c = FourCC::from(av01.av1c.box_type());
        assert_eq!(&fcc_c.value, b"av1C");

        // Write av1C and check exact bytes
        let mut av1c_buf = Vec::new();
        av01.av1c.write_box(&mut av1c_buf).unwrap();

        // 8 bytes box header + 4 bytes fixed record + 6 bytes configOBUs = 18 bytes
        assert_eq!(av1c_buf.len(), 18);
        assert_eq!(av01.av1c.box_size(), 18);

        let record = &av1c_buf[8..8 + 4];
        // byte 0: marker=1, version=1 -> 0x81
        assert_eq!(record[0], 0x81);
        // byte 1: seq_profile=0, seq_level_idx_0=8 -> (0 << 5) | 8 = 0x08
        assert_eq!(record[1], 0x08);
        // byte 2: chroma_subsampling_x=1, chroma_subsampling_y=1 -> 0x08 | 0x04 = 0x0C
        assert_eq!(record[2], 0x0C);
        // byte 3: delay present (0x10) | delay_minus_one(2) = 0x12
        assert_eq!(record[3], 0x12);

        // configOBUs follow exactly
        assert_eq!(&av1c_buf[12..], config.config_obus.as_slice());

        // Full av01 roundtrip
        let mut av01_buf = Vec::new();
        av01.write_box(&mut av01_buf).unwrap();
        assert_eq!(av01_buf.len(), av01.box_size() as usize);

        let mut reader = Cursor::new(&av01_buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(header.name, BoxType::Av01Box);
        let parsed = Av01Box::read_box(&mut reader, header.size).unwrap();
        assert_eq!(av01, parsed);
    }

    #[test]
    fn test_av1_validation_and_obu_parser_rejections() {
        // Zero dimensions
        let mut cfg = sample_av1_config();
        cfg.width = 0;
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Invalid seq_profile (> 7)
        let mut cfg = sample_av1_config();
        cfg.seq_profile = 8;
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Invalid seq_level_idx_0 (> 31)
        let mut cfg = sample_av1_config();
        cfg.seq_level_idx_0 = 32;
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // twelve_bit without high_bitdepth
        let mut cfg = sample_av1_config();
        cfg.twelve_bit = true;
        cfg.high_bitdepth = false;
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // chroma_sample_position > 3
        let mut cfg = sample_av1_config();
        cfg.chroma_sample_position = 4;
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // initial_presentation_delay_minus_one > 15
        let mut cfg = sample_av1_config();
        cfg.initial_presentation_delay_minus_one = Some(16);
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Empty config_obus
        let mut cfg = sample_av1_config();
        cfg.config_obus = vec![];
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // OBU with forbidden bit set (bit 7)
        let mut cfg = sample_av1_config();
        cfg.config_obus = vec![0x8A, 0x04, 0x20, 0x00, 0x00, 0x00];
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // OBU missing size field (obu_has_size_field=0, e.g. header 0x08 instead of 0x0A)
        let mut cfg = sample_av1_config();
        cfg.config_obus = vec![0x08, 0x20, 0x00, 0x00, 0x00];
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // No Sequence Header OBU (e.g. only Temporal Delimiter type 2: (2 << 3) | 2 = 0x12)
        let mut cfg = sample_av1_config();
        cfg.config_obus = vec![0x12, 0x00];
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Truncated OBU payload
        let mut cfg = sample_av1_config();
        cfg.config_obus = vec![0x0A, 0x05, 0x20, 0x00]; // claims size 5, only 2 provided
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Truncated extension header (obu_extension_flag=1 -> 0x0E with no extension byte)
        let mut cfg = sample_av1_config();
        cfg.config_obus = vec![0x0E];
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Truncated LEB128 size (continuation bit set at EOF)
        let mut cfg = sample_av1_config();
        cfg.config_obus = vec![0x0A, 0x80];
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // LEB128 exceeding 8 bytes
        let mut cfg = sample_av1_config();
        cfg.config_obus = vec![
            0x0A, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00, 0x00,
        ];
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));
        // OBU with reserved bit 0 set (bit 0)
        let mut cfg = sample_av1_config();
        cfg.config_obus = vec![0x0B, 0x04, 0x20, 0x00, 0x00, 0x00];
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Extension header with reserved bits set (bits 2..0 != 0)
        let mut cfg = sample_av1_config();
        cfg.config_obus = vec![0x0E, 0x01, 0x04, 0x20, 0x00, 0x00, 0x00];
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Non-minimal LEB128 in config_obus (e.g. 0x84, 0x00 instead of 0x04)
        let mut cfg = sample_av1_config();
        cfg.config_obus = vec![0x0A, 0x84, 0x00, 0x20, 0x00, 0x00, 0x00];
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Non-permitted configOBU type (Padding OBU type 15 in config)
        let mut cfg = sample_av1_config();
        cfg.config_obus = vec![
            0x0A, 0x04, 0x20, 0x00, 0x00, 0x00, // Seq Header
            0x7A, 0x02, 0x00, 0x00, // Padding OBU (not allowed in configOBUs)
        ];
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Duplicate Sequence Header in config_obus
        let mut cfg = sample_av1_config();
        cfg.config_obus = vec![
            0x0A, 0x04, 0x20, 0x00, 0x00, 0x00, // Seq Header 1
            0x0A, 0x04, 0x20, 0x00, 0x00, 0x00, // Duplicate Seq Header
        ];
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Sequence Header not first
        let mut cfg = sample_av1_config();
        cfg.config_obus = vec![
            0x2A, 0x02, 0x00, 0x00, // Metadata OBU first
            0x0A, 0x04, 0x20, 0x00, 0x00, 0x00, // Seq Header second
        ];
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Invalid seq_profile (3)
        let mut cfg = sample_av1_config();
        cfg.seq_profile = 3;
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Reserved seq_level_idx_0 (20)
        let mut cfg = sample_av1_config();
        cfg.seq_level_idx_0 = 20;
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // High tier at level <= 7
        let mut cfg = sample_av1_config();
        cfg.seq_level_idx_0 = 7;
        cfg.seq_tier_0 = true;
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Chroma sample position 3
        let mut cfg = sample_av1_config();
        cfg.chroma_sample_position = 3;
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Profile 0 with 4:4:4
        let mut cfg = sample_av1_config();
        cfg.seq_profile = 0;
        cfg.chroma_subsampling_x = false;
        cfg.chroma_subsampling_y = false;
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Profile 1 with monochrome
        let mut cfg = sample_av1_config();
        cfg.seq_profile = 1;
        cfg.monochrome = true;
        cfg.chroma_subsampling_x = false;
        cfg.chroma_subsampling_y = false;
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Profile 0 with twelve_bit
        let mut cfg = sample_av1_config();
        cfg.seq_profile = 0;
        cfg.high_bitdepth = true;
        cfg.twelve_bit = true;
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));

        // Profile 2 12-bit with x=false, y=true
        let mut cfg = sample_av1_config();
        cfg.seq_profile = 2;
        cfg.high_bitdepth = true;
        cfg.twelve_bit = true;
        cfg.chroma_subsampling_x = false;
        cfg.chroma_subsampling_y = true;
        assert!(matches!(Av01Box::new(&cfg), Err(Error::InvalidData(_))));
    }

    #[test]
    fn test_av1_multiple_obus_in_config() {
        let mut cfg = sample_av1_config();
        // Sequence Header (type 1, len 4) + Metadata OBU (type 5: (5 << 3) | 2 = 0x2A, len 2)
        cfg.config_obus = vec![
            0x0A, 0x04, 0x20, 0x00, 0x00, 0x00, // Seq Header
            0x2A, 0x02, 0x00, 0x00, // Metadata OBU
        ];
        let av01 = Av01Box::new(&cfg).unwrap();

        let mut buf = Vec::new();
        av01.write_box(&mut buf).unwrap();

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        let parsed = Av01Box::read_box(&mut reader, header.size).unwrap();
        assert_eq!(av01, parsed);
        assert_eq!(parsed.av1c.config_obus.len(), 10);
    }

    #[test]
    fn test_av1c_read_rejections() {
        // Invalid marker/version in av1C box (e.g. byte 0 is 0x01 instead of 0x81)
        let invalid_record = vec![
            0x00, 0x00, 0x00, 0x0E, // size 14
            0x61, 0x76, 0x31, 0x43, // 'av1C'
            0x01, // marker=0, version=1 (invalid marker)
            0x00, 0x00, 0x00, 0x0A, 0x00,
        ];
        let mut reader = Cursor::new(&invalid_record);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert!(matches!(
            Av1CBox::read_box(&mut reader, header.size),
            Err(Error::InvalidData(_))
        ));

        // av1C size too small (< 12 bytes including header)
        let too_small = vec![
            0x00, 0x00, 0x00, 0x0B, // size 11
            0x61, 0x76, 0x31, 0x43, // 'av1C'
            0x81, 0x00, 0x00,
        ];
        let mut reader = Cursor::new(&too_small);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert!(matches!(
            Av1CBox::read_box(&mut reader, header.size),
            Err(Error::InvalidData(_))
        ));
    }
}
