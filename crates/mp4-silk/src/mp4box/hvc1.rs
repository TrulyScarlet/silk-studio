use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::Serialize;
use std::convert::TryFrom;
use std::io::{Read, Seek, Write};

use crate::mp4box::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HevcNalArray {
    pub array_completeness: u8,
    pub nal_unit_type: u8,
    pub nal_units: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HvcCBox {
    pub configuration_version: u8,
    pub general_profile_space: u8,
    pub general_tier_flag: bool,
    pub general_profile_idc: u8,
    pub general_profile_compatibility_flags: u32,
    pub general_constraint_indicator_flags: u64,
    pub general_level_idc: u8,
    pub min_spatial_segmentation_idc: u16,
    pub parallelism_type: u8,
    pub chroma_format_idc: u8,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub avg_frame_rate: u16,
    pub constant_frame_rate: u8,
    pub num_temporal_layers: u8,
    pub temporal_id_nested: bool,
    pub length_size_minus_one: u8,
    pub arrays: Vec<HevcNalArray>,
}

impl Default for HvcCBox {
    fn default() -> Self {
        Self {
            configuration_version: 1,
            general_profile_space: 0,
            general_tier_flag: false,
            general_profile_idc: 1,
            general_profile_compatibility_flags: 0x6000_0000,
            general_constraint_indicator_flags: 0,
            general_level_idc: 120,
            min_spatial_segmentation_idc: 0,
            parallelism_type: 0,
            chroma_format_idc: 1,
            bit_depth_luma_minus8: 0,
            bit_depth_chroma_minus8: 0,
            avg_frame_rate: 0,
            constant_frame_rate: 0,
            num_temporal_layers: 1,
            temporal_id_nested: true,
            length_size_minus_one: 3,
            arrays: Vec::new(),
        }
    }
}

impl TryFrom<&HevcConfig> for HvcCBox {
    type Error = Error;

    fn try_from(config: &HevcConfig) -> Result<Self> {
        config.validate()?;

        let arrays = vec![
            HevcNalArray {
                array_completeness: 1,
                nal_unit_type: 32, // VPS
                nal_units: config.vps.clone(),
            },
            HevcNalArray {
                array_completeness: 1,
                nal_unit_type: 33, // SPS
                nal_units: config.sps.clone(),
            },
            HevcNalArray {
                array_completeness: 1,
                nal_unit_type: 34, // PPS
                nal_units: config.pps.clone(),
            },
        ];

        Ok(HvcCBox {
            configuration_version: 1,
            general_profile_space: config.general_profile_space,
            general_tier_flag: config.general_tier_flag,
            general_profile_idc: config.general_profile_idc,
            general_profile_compatibility_flags: config.general_profile_compatibility_flags,
            general_constraint_indicator_flags: config.general_constraint_indicator_flags,
            general_level_idc: config.general_level_idc,
            min_spatial_segmentation_idc: config.min_spatial_segmentation_idc,
            parallelism_type: config.parallelism_type,
            chroma_format_idc: config.chroma_format_idc,
            bit_depth_luma_minus8: config.bit_depth_luma_minus8,
            bit_depth_chroma_minus8: config.bit_depth_chroma_minus8,
            avg_frame_rate: config.avg_frame_rate,
            constant_frame_rate: config.constant_frame_rate,
            num_temporal_layers: config.num_temporal_layers,
            temporal_id_nested: config.temporal_id_nested,
            length_size_minus_one: config.length_size_minus_one,
            arrays,
        })
    }
}

impl TryFrom<HevcConfig> for HvcCBox {
    type Error = Error;

    fn try_from(config: HevcConfig) -> Result<Self> {
        Self::try_from(&config)
    }
}

impl HvcCBox {
    pub fn new(config: &HevcConfig) -> Result<Self> {
        Self::try_from(config)
    }
}

impl Mp4Box for HvcCBox {
    fn box_type(&self) -> BoxType {
        BoxType::HvcCBox
    }

    fn box_size(&self) -> u64 {
        let mut size: u64 = HEADER_SIZE + 23;
        for array in &self.arrays {
            size = size
                .checked_add(3)
                .expect("hvcC array header size overflow");
            for nal in &array.nal_units {
                let nal_entry_size = (2usize)
                    .checked_add(nal.len())
                    .expect("hvcC nal size overflow");
                size = size
                    .checked_add(nal_entry_size as u64)
                    .expect("hvcC box size overflow");
            }
        }
        size
    }

    fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(&self).unwrap())
    }

    fn summary(&self) -> Result<String> {
        let s = format!(
            "configuration_version={} profile_idc={} level_idc={} num_arrays={}",
            self.configuration_version,
            self.general_profile_idc,
            self.general_level_idc,
            self.arrays.len()
        );
        Ok(s)
    }
}

impl<R: Read + Seek> ReadBox<&mut R> for HvcCBox {
    fn read_box(reader: &mut R, size: u64) -> Result<Self> {
        let start = box_start(reader)?;

        if size < HEADER_SIZE + 23 {
            return Err(Error::InvalidData("hvcC box size too small"));
        }

        let configuration_version = reader.read_u8()?;
        if configuration_version != 1 {
            return Err(Error::InvalidData("unsupported HEVC configuration version"));
        }

        let byte1 = reader.read_u8()?;
        let general_profile_space = (byte1 >> 6) & 0x03;
        let general_tier_flag = (byte1 & 0x20) != 0;
        let general_profile_idc = byte1 & 0x1F;

        let general_profile_compatibility_flags = reader.read_u32::<BigEndian>()?;

        let mut constraint_bytes = [0u8; 8];
        reader.read_exact(&mut constraint_bytes[2..8])?;
        let general_constraint_indicator_flags = u64::from_be_bytes(constraint_bytes);

        let general_level_idc = reader.read_u8()?;

        let min_spatial_segmentation_raw = reader.read_u16::<BigEndian>()?;
        let min_spatial_segmentation_idc = min_spatial_segmentation_raw & 0x0FFF;

        let parallelism_type = reader.read_u8()? & 0x03;
        let chroma_format_idc = reader.read_u8()? & 0x03;
        let bit_depth_luma_minus8 = reader.read_u8()? & 0x07;
        let bit_depth_chroma_minus8 = reader.read_u8()? & 0x07;
        let avg_frame_rate = reader.read_u16::<BigEndian>()?;

        let byte21 = reader.read_u8()?;
        let constant_frame_rate = (byte21 >> 6) & 0x03;
        let num_temporal_layers = (byte21 >> 3) & 0x07;
        let temporal_id_nested = (byte21 & 0x04) != 0;
        let length_size_minus_one = byte21 & 0x03;

        let num_of_arrays = reader.read_u8()?;
        let mut arrays = Vec::with_capacity(num_of_arrays as usize);

        for _ in 0..num_of_arrays {
            let current_pos = reader.stream_position()?;
            if current_pos - start > size {
                return Err(Error::InvalidData("hvcC box read overflow"));
            }
            let array_header = reader.read_u8()?;
            let array_completeness = (array_header >> 7) & 0x01;
            let nal_unit_type = array_header & 0x3F;
            let num_nalus = reader.read_u16::<BigEndian>()?;
            let mut nal_units = Vec::with_capacity(num_nalus as usize);
            for _ in 0..num_nalus {
                let nal_length = reader.read_u16::<BigEndian>()? as usize;
                let pos_before_nal = reader.stream_position()?;
                if (pos_before_nal - start).saturating_add(nal_length as u64) > size {
                    return Err(Error::InvalidData("hvcC NAL unit exceeds box size"));
                }
                let mut nal_bytes = vec![0u8; nal_length];
                reader.read_exact(&mut nal_bytes)?;
                nal_units.push(nal_bytes);
            }
            arrays.push(HevcNalArray {
                array_completeness,
                nal_unit_type,
                nal_units,
            });
        }

        skip_bytes_to(reader, start + size)?;

        Ok(HvcCBox {
            configuration_version,
            general_profile_space,
            general_tier_flag,
            general_profile_idc,
            general_profile_compatibility_flags,
            general_constraint_indicator_flags,
            general_level_idc,
            min_spatial_segmentation_idc,
            parallelism_type,
            chroma_format_idc,
            bit_depth_luma_minus8,
            bit_depth_chroma_minus8,
            avg_frame_rate,
            constant_frame_rate,
            num_temporal_layers,
            temporal_id_nested,
            length_size_minus_one,
            arrays,
        })
    }
}

impl<W: Write> WriteBox<&mut W> for HvcCBox {
    fn write_box(&self, writer: &mut W) -> Result<u64> {
        let size = self.box_size();
        BoxHeader::new(self.box_type(), size).write(writer)?;

        writer.write_u8(self.configuration_version)?;
        let byte1 = (self.general_profile_space << 6)
            | (if self.general_tier_flag { 0x20 } else { 0 })
            | (self.general_profile_idc & 0x1F);
        writer.write_u8(byte1)?;
        writer.write_u32::<BigEndian>(self.general_profile_compatibility_flags)?;

        let constraint_bytes = self.general_constraint_indicator_flags.to_be_bytes();
        writer.write_all(&constraint_bytes[2..8])?;

        writer.write_u8(self.general_level_idc)?;
        writer.write_u16::<BigEndian>(0xF000 | (self.min_spatial_segmentation_idc & 0x0FFF))?;
        writer.write_u8(0xFC | (self.parallelism_type & 0x03))?;
        writer.write_u8(0xFC | (self.chroma_format_idc & 0x03))?;
        writer.write_u8(0xF8 | (self.bit_depth_luma_minus8 & 0x07))?;
        writer.write_u8(0xF8 | (self.bit_depth_chroma_minus8 & 0x07))?;
        writer.write_u16::<BigEndian>(self.avg_frame_rate)?;

        let byte21 = ((self.constant_frame_rate & 0x03) << 6)
            | ((self.num_temporal_layers & 0x07) << 3)
            | (if self.temporal_id_nested { 0x04 } else { 0 })
            | (self.length_size_minus_one & 0x03);
        writer.write_u8(byte21)?;

        if self.arrays.len() > u8::MAX as usize {
            return Err(Error::InvalidData("too many arrays in hvcC"));
        }
        writer.write_u8(self.arrays.len() as u8)?;
        for array in &self.arrays {
            let array_header =
                ((array.array_completeness & 0x01) << 7) | (array.nal_unit_type & 0x3F);
            writer.write_u8(array_header)?;
            if array.nal_units.len() > u16::MAX as usize {
                return Err(Error::InvalidData("too many NAL units in array"));
            }
            writer.write_u16::<BigEndian>(array.nal_units.len() as u16)?;
            for nal in &array.nal_units {
                if nal.len() > u16::MAX as usize {
                    return Err(Error::InvalidData("NAL unit length exceeds 65535 bytes"));
                }
                writer.write_u16::<BigEndian>(nal.len() as u16)?;
                writer.write_all(nal)?;
            }
        }

        Ok(size)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hvc1Box {
    pub data_reference_index: u16,
    pub width: u16,
    pub height: u16,

    #[serde(with = "value_u32")]
    pub horizresolution: FixedPointU16,

    #[serde(with = "value_u32")]
    pub vertresolution: FixedPointU16,
    pub frame_count: u16,
    pub depth: u16,
    pub hvcc: HvcCBox,
}

impl Default for Hvc1Box {
    fn default() -> Self {
        Hvc1Box {
            data_reference_index: 0,
            width: 0,
            height: 0,
            horizresolution: FixedPointU16::new(0x48),
            vertresolution: FixedPointU16::new(0x48),
            frame_count: 1,
            depth: 0x0018,
            hvcc: HvcCBox::default(),
        }
    }
}

impl Hvc1Box {
    pub fn new(config: &HevcConfig) -> Result<Self> {
        let hvcc = HvcCBox::try_from(config)?;
        Ok(Hvc1Box {
            data_reference_index: 1,
            width: config.width,
            height: config.height,
            horizresolution: FixedPointU16::new(0x48),
            vertresolution: FixedPointU16::new(0x48),
            frame_count: 1,
            depth: 0x0018,
            hvcc,
        })
    }

    pub fn get_type(&self) -> BoxType {
        BoxType::Hvc1Box
    }

    pub fn get_size(&self) -> u64 {
        HEADER_SIZE + 8 + 70 + self.hvcc.box_size()
    }
}

impl TryFrom<&HevcConfig> for Hvc1Box {
    type Error = Error;

    fn try_from(config: &HevcConfig) -> Result<Self> {
        Self::new(config)
    }
}

impl TryFrom<HevcConfig> for Hvc1Box {
    type Error = Error;

    fn try_from(config: HevcConfig) -> Result<Self> {
        Self::new(&config)
    }
}

impl Mp4Box for Hvc1Box {
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

impl<R: Read + Seek> ReadBox<&mut R> for Hvc1Box {
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
                "hvc1 box contains a box with a larger size than it",
            ));
        }
        if name == BoxType::HvcCBox {
            let hvcc = HvcCBox::read_box(reader, s)?;

            skip_bytes_to(reader, start + size)?;

            Ok(Hvc1Box {
                data_reference_index,
                width,
                height,
                horizresolution,
                vertresolution,
                frame_count,
                depth,
                hvcc,
            })
        } else {
            Err(Error::InvalidData("hvcc not found"))
        }
    }
}

impl<W: Write> WriteBox<&mut W> for Hvc1Box {
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

        self.hvcc.write_box(writer)?;

        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4box::BoxHeader;
    use std::io::Cursor;

    fn sample_hevc_config() -> HevcConfig {
        HevcConfig {
            width: 1920,
            height: 1080,
            general_profile_space: 0,
            general_tier_flag: false,
            general_profile_idc: 1, // Main profile
            general_profile_compatibility_flags: 0x6000_0000,
            general_constraint_indicator_flags: 0x9000_0000_0000,
            general_level_idc: 120, // Level 4.0
            min_spatial_segmentation_idc: 0,
            parallelism_type: 0,
            chroma_format_idc: 1, // 4:2:0
            bit_depth_luma_minus8: 0,
            bit_depth_chroma_minus8: 0,
            avg_frame_rate: 60,
            constant_frame_rate: 1,
            num_temporal_layers: 1,
            temporal_id_nested: true,
            length_size_minus_one: 3,
            vps: vec![vec![
                0x40, 0x01, 0x0c, 0x01, 0xff, 0xff, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x80, 0x00,
                0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x78, 0xac, 0x09,
            ]],
            sps: vec![vec![
                0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x80, 0x00, 0x00, 0x03, 0x00,
                0x00, 0x03, 0x00, 0x78, 0xa0, 0x05, 0x02, 0x01, 0x48, 0x59, 0x5a, 0x6e, 0x4f, 0x01,
                0x0e, 0x01, 0x11, 0x72, 0x74, 0x08, 0x00, 0x00, 0x03, 0x00, 0x08, 0x00, 0x00, 0x03,
                0x01, 0xe0, 0x40,
            ]],
            pps: vec![vec![0x44, 0x01, 0xc1, 0x72, 0xb4, 0x62, 0x40]],
        }
    }

    #[test]
    fn test_hvc1_sample_entry_fourcc_and_roundtrip() {
        let config = sample_hevc_config();
        let hvc1 = Hvc1Box::new(&config).unwrap();

        // Emitted sample entry FourCC is exactly hvc1
        assert_eq!(hvc1.box_type(), BoxType::Hvc1Box);
        let fcc = FourCC::from(hvc1.box_type());
        assert_eq!(&fcc.value, b"hvc1");

        let mut buf = Vec::new();
        hvc1.write_box(&mut buf).unwrap();
        assert_eq!(buf.len(), hvc1.box_size() as usize);

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(header.name, BoxType::Hvc1Box);
        assert_eq!(hvc1.box_size(), header.size);

        let parsed = Hvc1Box::read_box(&mut reader, header.size).unwrap();
        assert_eq!(hvc1, parsed);
        assert_eq!(parsed.width, 1920);
        assert_eq!(parsed.height, 1080);
    }

    #[test]
    fn test_hvcc_fixed_header_23_bytes_and_length_size() {
        let config = sample_hevc_config();
        let hvcc = HvcCBox::try_from(&config).unwrap();

        let mut buf = Vec::new();
        hvcc.write_box(&mut buf).unwrap();
        assert_eq!(buf.len(), hvcc.box_size() as usize);

        // Header size is 8 bytes, followed by 23-byte fixed HEVCDecoderConfigurationRecord
        let fixed_record = &buf[8..8 + 23];
        assert_eq!(fixed_record.len(), 23);

        // byte 0: configurationVersion = 1
        assert_eq!(fixed_record[0], 1);

        // byte 1: general_profile_space(0), general_tier_flag(0), general_profile_idc(1)
        assert_eq!(fixed_record[1], 1);

        // bytes 2..6: compatibility flags = 0x6000_0000
        let compat = u32::from_be_bytes([
            fixed_record[2],
            fixed_record[3],
            fixed_record[4],
            fixed_record[5],
        ]);
        assert_eq!(compat, 0x6000_0000);

        // bytes 6..12: 48-bit constraint flags = 0x9000_0000_0000
        let constraint = u64::from_be_bytes([
            0,
            0,
            fixed_record[6],
            fixed_record[7],
            fixed_record[8],
            fixed_record[9],
            fixed_record[10],
            fixed_record[11],
        ]);
        assert_eq!(constraint, 0x9000_0000_0000);

        // byte 12: level = 120
        assert_eq!(fixed_record[12], 120);

        // byte 21: constantFrameRate(1 << 6), numTemporalLayers(1 << 3), temporalIdNested(1 << 2), lengthSizeMinusOne(3)
        let byte21 = fixed_record[21];
        let length_size_minus_one = byte21 & 0x03;
        assert_eq!(length_size_minus_one, 3);
        assert_eq!((byte21 >> 6) & 0x03, 1);
        assert_eq!((byte21 >> 3) & 0x07, 1);
        assert_eq!((byte21 >> 2) & 0x01, 1);

        // byte 22: numOfArrays = 3
        assert_eq!(fixed_record[22], 3);

        // Arrays follow immediately after 23-byte fixed header
        let array_data = &buf[8 + 23..];

        // Array 0: VPS (type 32 = 0x20, completeness = 1 -> 0xA0)
        assert_eq!(array_data[0], 0xA0);
        let vps_count = u16::from_be_bytes([array_data[1], array_data[2]]);
        assert_eq!(vps_count, 1);
        let vps_len = u16::from_be_bytes([array_data[3], array_data[4]]) as usize;
        assert_eq!(vps_len, config.vps[0].len());
        assert_eq!(&array_data[5..5 + vps_len], config.vps[0].as_slice());

        let offset1 = 5 + vps_len;
        // Array 1: SPS (type 33 = 0x21, completeness = 1 -> 0xA1)
        assert_eq!(array_data[offset1], 0xA1);
        let sps_count = u16::from_be_bytes([array_data[offset1 + 1], array_data[offset1 + 2]]);
        assert_eq!(sps_count, 1);
        let sps_len =
            u16::from_be_bytes([array_data[offset1 + 3], array_data[offset1 + 4]]) as usize;
        assert_eq!(sps_len, config.sps[0].len());
        assert_eq!(
            &array_data[offset1 + 5..offset1 + 5 + sps_len],
            config.sps[0].as_slice()
        );

        let offset2 = offset1 + 5 + sps_len;
        // Array 2: PPS (type 34 = 0x22, completeness = 1 -> 0xA2)
        assert_eq!(array_data[offset2], 0xA2);
        let pps_count = u16::from_be_bytes([array_data[offset2 + 1], array_data[offset2 + 2]]);
        assert_eq!(pps_count, 1);
        let pps_len =
            u16::from_be_bytes([array_data[offset2 + 3], array_data[offset2 + 4]]) as usize;
        assert_eq!(pps_len, config.pps[0].len());
        assert_eq!(
            &array_data[offset2 + 5..offset2 + 5 + pps_len],
            config.pps[0].as_slice()
        );

        // Roundtrip
        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(header.name, BoxType::HvcCBox);
        let parsed = HvcCBox::read_box(&mut reader, header.size).unwrap();
        assert_eq!(hvcc, parsed);
    }

    #[test]
    fn test_hevc_multiple_parameter_sets() {
        let mut config = sample_hevc_config();
        config.sps.push(vec![0x42, 0x01, 0x01, 0x99]);
        config.pps.push(vec![0x44, 0x01, 0xaa, 0xbb]);

        let hvcc = HvcCBox::try_from(&config).unwrap();
        assert_eq!(hvcc.arrays.len(), 3);
        assert_eq!(hvcc.arrays[0].nal_units.len(), 1);
        assert_eq!(hvcc.arrays[1].nal_units.len(), 2);
        assert_eq!(hvcc.arrays[2].nal_units.len(), 2);

        let mut buf = Vec::new();
        hvcc.write_box(&mut buf).unwrap();

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        let parsed = HvcCBox::read_box(&mut reader, header.size).unwrap();
        assert_eq!(hvcc, parsed);
        assert_eq!(parsed.arrays[1].nal_units.len(), 2);
        assert_eq!(parsed.arrays[2].nal_units.len(), 2);
    }

    #[test]
    fn test_hevc_validation_rejects_invalid_data() {
        // Test out of range general_profile_space (> 3)
        let mut cfg = sample_hevc_config();
        cfg.general_profile_space = 4;
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test out of range general_profile_idc (> 31)
        let mut cfg = sample_hevc_config();
        cfg.general_profile_idc = 32;
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test out of range general_constraint_indicator_flags (> 48-bit)
        let mut cfg = sample_hevc_config();
        cfg.general_constraint_indicator_flags = 1u64 << 48;
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test out of range min_spatial_segmentation_idc (> 4095)
        let mut cfg = sample_hevc_config();
        cfg.min_spatial_segmentation_idc = 4096;
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test out of range parallelism_type (> 3)
        let mut cfg = sample_hevc_config();
        cfg.parallelism_type = 4;
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test out of range chroma_format_idc (> 3)
        let mut cfg = sample_hevc_config();
        cfg.chroma_format_idc = 4;
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test out of range bit_depth_luma_minus8 (> 7)
        let mut cfg = sample_hevc_config();
        cfg.bit_depth_luma_minus8 = 8;
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test out of range bit_depth_chroma_minus8 (> 7)
        let mut cfg = sample_hevc_config();
        cfg.bit_depth_chroma_minus8 = 8;
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test out of range constant_frame_rate (> 3)
        let mut cfg = sample_hevc_config();
        cfg.constant_frame_rate = 4;
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test out of range num_temporal_layers (> 7)
        let mut cfg = sample_hevc_config();
        cfg.num_temporal_layers = 8;
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test out of range length_size_minus_one (!= 3)
        let mut cfg = sample_hevc_config();
        cfg.length_size_minus_one = 0;
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));
        let mut cfg = sample_hevc_config();
        cfg.length_size_minus_one = 4;
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test empty VPS array
        let mut cfg = sample_hevc_config();
        cfg.vps = vec![];
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test empty SPS array
        let mut cfg = sample_hevc_config();
        cfg.sps = vec![];
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test empty PPS array
        let mut cfg = sample_hevc_config();
        cfg.pps = vec![];
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test empty NAL unit
        let mut cfg = sample_hevc_config();
        cfg.vps = vec![vec![]];
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test oversized NAL unit (> 65535)
        let mut cfg = sample_hevc_config();
        cfg.vps = vec![vec![0u8; 65536]];
        assert!(matches!(
            HvcCBox::try_from(&cfg),
            Err(Error::InvalidData(_))
        ));

        // Test zero dimensions
        let mut cfg = sample_hevc_config();
        cfg.width = 0;
        assert!(matches!(Hvc1Box::new(&cfg), Err(Error::InvalidData(_))));
    }
}
