use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use audio_api::{
    AudioCapture, AudioCaptureConfig, AudioCaptureError, AudioCaptureEvent, AudioFormat, Result,
};
use media_types::{AudioDeviceInfo, AudioFrame, SampleFormat, TimeBase};
use windows::core::{implement, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_NOT_FOUND, HANDLE, PROPERTYKEY, RPC_E_CHANGED_MODE, S_FALSE, S_OK,
};
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, ERole, IAudioCaptureClient, IAudioClient, IMMDevice,
    IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator,
    AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_S_BUFFER_EMPTY, DEVICE_STATE, DEVICE_STATE_ACTIVE,
    WAVEFORMATEX,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Performance::QueryPerformanceCounter;
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};

const QPC_TIME_BASE: TimeBase = TimeBase::new(1, 10_000_000);
const WAIT_TIMEOUT: u32 = 258;
const WAIT_OBJECT_0: u32 = 0;
const QPC_POSITION_INVALID: u64 = u64::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasapiCaptureKind {
    Loopback,
    Microphone,
}

impl WasapiCaptureKind {
    fn flow(self) -> windows::Win32::Media::Audio::EDataFlow {
        match self {
            Self::Loopback => eRender,
            Self::Microphone => eCapture,
        }
    }

    fn device_kind(self) -> media_types::AudioDeviceKind {
        match self {
            Self::Loopback => media_types::AudioDeviceKind::Output,
            Self::Microphone => media_types::AudioDeviceKind::Input,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Loopback => "desktop output",
            Self::Microphone => "microphone input",
        }
    }
}

struct ComGuard {
    should_uninitialize: bool,
}

impl ComGuard {
    fn initialize() -> Result<Self> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr == S_OK || hr == S_FALSE {
            Ok(Self {
                should_uninitialize: true,
            })
        } else if hr == RPC_E_CHANGED_MODE {
            Ok(Self {
                should_uninitialize: false,
            })
        } else if let Err(error) = hr.ok() {
            Err(backend_error("initialize COM", error))
        } else {
            Ok(Self {
                should_uninitialize: true,
            })
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.should_uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

struct NotificationState {
    pending: AtomicBool,
    // Store the raw value rather than the windows-rs handle wrapper so this
    // callback state can safely cross the notification thread boundary.
    event: Option<usize>,
}

impl NotificationState {
    fn new(event: Option<HANDLE>) -> Self {
        Self {
            pending: AtomicBool::new(false),
            event: event.map(|handle| handle.0 as usize),
        }
    }

    fn signal(&self) {
        self.pending.store(true, Ordering::Release);
        if let Some(event) = self.event {
            unsafe {
                let _ = SetEvent(HANDLE(event as *mut core::ffi::c_void));
            }
        }
    }

    fn take_pending(&self) -> bool {
        self.pending.swap(false, Ordering::AcqRel)
    }
}

#[implement(IMMNotificationClient)]
struct EndpointNotificationClient {
    state: Arc<NotificationState>,
    kind: WasapiCaptureKind,
    follows_default: bool,
    selected_device_id: Vec<u16>,
}

impl EndpointNotificationClient {
    fn new(
        state: Arc<NotificationState>,
        kind: WasapiCaptureKind,
        follows_default: bool,
        selected_device_id: &str,
    ) -> Self {
        Self {
            state,
            kind,
            follows_default,
            selected_device_id: wide_null(selected_device_id),
        }
    }

    fn selected_device(&self, device_id: &PCWSTR) -> bool {
        pcwstr_matches(device_id, &self.selected_device_id)
    }
}

impl EndpointNotificationClient {
    fn on_device_state_changed(&self, device_id: &PCWSTR, new_state: DEVICE_STATE) {
        if self.selected_device(device_id) && new_state != DEVICE_STATE_ACTIVE {
            self.state.signal();
        }
    }

    fn on_device_removed(&self, device_id: &PCWSTR) {
        if self.selected_device(device_id) {
            self.state.signal();
        }
    }

    fn on_default_device_changed(&self, flow: EDataFlow, role: ERole, default_device_id: &PCWSTR) {
        if self.follows_default
            && flow == self.kind.flow()
            && role == eConsole
            && !self.selected_device(default_device_id)
        {
            self.state.signal();
        }
    }
}

impl IMMNotificationClient_Impl for EndpointNotificationClient_Impl {
    fn OnDeviceStateChanged(
        &self,
        device_id: &PCWSTR,
        new_state: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        self.on_device_state_changed(device_id, new_state);
        Ok(())
    }

    fn OnDeviceAdded(&self, _device_id: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, device_id: &PCWSTR) -> windows::core::Result<()> {
        self.on_device_removed(device_id);
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        default_device_id: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.on_default_device_changed(flow, role, default_device_id);
        Ok(())
    }

    fn OnPropertyValueChanged(
        &self,
        _device_id: &PCWSTR,
        _key: &PROPERTYKEY,
    ) -> windows::core::Result<()> {
        Ok(())
    }
}

struct CaptureState {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    enumerator: IMMDeviceEnumerator,
    notification_client: IMMNotificationClient,
    notification: Arc<NotificationState>,
    event: HANDLE,
    format: AudioFormat,
    block_align: usize,
    // Declared last so COM interface wrappers release before CoUninitialize.
    _com: ComGuard,
}

impl Drop for CaptureState {
    fn drop(&mut self) {
        unsafe {
            let _ = self
                .enumerator
                .UnregisterEndpointNotificationCallback(&self.notification_client);
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}

/// Event-driven shared-mode WASAPI capture for either system loopback or a
/// microphone endpoint. The object is handed to one worker before `start` and
/// is never accessed concurrently; the unsafe `Send` marker preserves that
/// ownership rule for the COM bindings.
pub struct WasapiAudioCapture {
    kind: WasapiCaptureKind,
    stream_id: media_types::StreamId,
    state: Option<CaptureState>,
    format: Option<AudioFormat>,
}

unsafe impl Send for WasapiAudioCapture {}

impl WasapiAudioCapture {
    pub fn new(kind: WasapiCaptureKind) -> Self {
        Self {
            kind,
            stream_id: media_types::StreamId(0),
            state: None,
            format: None,
        }
    }

    pub fn with_stream_id(mut self, stream_id: media_types::StreamId) -> Self {
        self.stream_id = stream_id;
        self
    }

    pub fn loopback() -> Self {
        Self::new(WasapiCaptureKind::Loopback)
    }

    pub fn microphone() -> Self {
        Self::new(WasapiCaptureKind::Microphone)
    }
}

pub fn enumerate_devices(kind: WasapiCaptureKind) -> Result<Vec<AudioDeviceInfo>> {
    let _com = ComGuard::initialize()?;
    let enumerator = create_enumerator()?;
    let collection = unsafe {
        enumerator
            .EnumAudioEndpoints(kind.flow(), DEVICE_STATE_ACTIVE)
            .map_err(|error| backend_error("enumerate audio endpoints", error))?
    };
    let count = unsafe {
        collection
            .GetCount()
            .map_err(|error| backend_error("count audio endpoints", error))?
    };
    let mut devices = Vec::with_capacity(count as usize);
    for index in 0..count {
        let device = unsafe {
            collection
                .Item(index)
                .map_err(|error| backend_error("read audio endpoint", error))?
        };
        let id = device_id(&device)?;
        let name = device_friendly_name(&device)
            .unwrap_or_else(|| format!("{} {}", kind.label(), index + 1));
        devices.push(AudioDeviceInfo {
            name,
            id,
            kind: kind.device_kind(),
            is_default: false,
        });
    }
    if let Ok(default) = default_device(&enumerator, kind) {
        let default_id = device_id(&default)?;
        for device in &mut devices {
            device.is_default = device.id == default_id;
        }
    }
    Ok(devices)
}

impl AudioCapture for WasapiAudioCapture {
    fn enumerate_devices(&self) -> Result<Vec<AudioDeviceInfo>> {
        enumerate_devices(self.kind)
    }

    fn start(&mut self, config: AudioCaptureConfig) -> Result<()> {
        if self.state.is_some() {
            return Err(AudioCaptureError::InvalidConfiguration {
                reason: "audio capture is already started".to_string(),
            });
        }

        let com = ComGuard::initialize()?;
        let enumerator = create_enumerator()?;
        let device = select_device(&enumerator, self.kind, config.device_id.as_deref())?;
        let follows_default = config
            .device_id
            .as_deref()
            .map(|id| id.is_empty() || id == "default")
            .unwrap_or(true);
        let selected_device_id = device_id(&device)?;
        let client: IAudioClient = unsafe {
            device
                .Activate(CLSCTX_ALL, None)
                .map_err(|error| backend_error("activate audio client", error))?
        };
        let format_ptr = unsafe {
            client
                .GetMixFormat()
                .map_err(|error| backend_error("get audio mix format", error))?
        };
        let format_result = unsafe { parse_format(&*format_ptr) };
        let (format, block_align) = match format_result {
            Ok(format) => format,
            Err(error) => {
                unsafe {
                    CoTaskMemFree(Some(format_ptr.cast()));
                }
                return Err(error);
            }
        };

        let mut flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
        if self.kind == WasapiCaptureKind::Loopback {
            flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
        }
        let initialize_result =
            unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, format_ptr, None) }
                .map_err(|error| backend_error("initialize audio client", error));
        unsafe {
            CoTaskMemFree(Some(format_ptr.cast()));
        }
        initialize_result?;
        let event = unsafe { CreateEventW(None, false, false, None) }
            .map_err(|error| backend_error("create audio event", error))?;
        if let Err(error) = unsafe { client.SetEventHandle(event) } {
            unsafe {
                let _ = CloseHandle(event);
            }
            return Err(backend_error("set audio event", error));
        }
        let notification = Arc::new(NotificationState::new(Some(event)));
        let notification_impl = EndpointNotificationClient::new(
            Arc::clone(&notification),
            self.kind,
            follows_default,
            &selected_device_id,
        );
        let notification_client: IMMNotificationClient = notification_impl.into();
        if let Err(error) =
            unsafe { enumerator.RegisterEndpointNotificationCallback(&notification_client) }
        {
            unsafe {
                let _ = CloseHandle(event);
            }
            return Err(backend_error(
                "register audio endpoint notifications",
                error,
            ));
        }
        let capture: IAudioCaptureClient = unsafe {
            client.GetService().map_err(|error| {
                let _ = enumerator.UnregisterEndpointNotificationCallback(&notification_client);
                let _ = CloseHandle(event);
                backend_error("get audio capture service", error)
            })?
        };
        if let Err(error) = unsafe { client.Start() } {
            unsafe {
                let _ = enumerator.UnregisterEndpointNotificationCallback(&notification_client);
                let _ = CloseHandle(event);
            }
            return Err(backend_error("start audio client", error));
        }

        self.format = Some(format);
        self.state = Some(CaptureState {
            client,
            capture,
            enumerator,
            notification_client,
            notification,
            event,
            format,
            block_align,
            _com: com,
        });
        diagnostics::info(
            "wasapi",
            &format!(
                "{} capture started at {} Hz / {} channels",
                self.kind.label(),
                format.sample_rate,
                format.channels
            ),
        );
        Ok(())
    }

    fn next_event(&mut self) -> Result<AudioCaptureEvent> {
        if self.state.is_none() {
            return Err(AudioCaptureError::Backend {
                details: "audio capture is not started".to_string(),
            });
        }
        loop {
            if self
                .state
                .as_ref()
                .expect("state")
                .notification
                .take_pending()
            {
                self.state.take();
                self.format = None;
                return Ok(AudioCaptureEvent::DeviceLost);
            }
            let wait =
                unsafe { WaitForSingleObject(self.state.as_ref().expect("state").event, 50) };
            if wait.0 == WAIT_TIMEOUT {
                // When an endpoint is silent (especially WASAPI loopback where Windows halts buffer events
                // when no application is actively rendering audio), synthesize a silent audio frame for the
                // elapsed interval. This keeps the stream clock continuous and prevents timeline drift,
                // discontinuity errors, voice pacing compression, and missing tracks in the replay buffer.
                if let Some(state) = self.state.as_mut() {
                    let frames = (state.format.sample_rate / 20).max(1); // 50ms of frames
                    let byte_len = (frames as usize) * state.block_align;
                    let payload = vec![0_u8; byte_len];
                    let mut qpc = 0_i64;
                    let _ = unsafe { QueryPerformanceCounter(&mut qpc) };
                    return Ok(AudioCaptureEvent::Frames(AudioFrame {
                        stream_id: media_types::StreamId(0),
                        sample_format: state.format.sample_format,
                        sample_rate: state.format.sample_rate,
                        channels: state.format.channels,
                        sample_count: frames,
                        pts: qpc,
                        time_base: QPC_TIME_BASE,
                        data: Arc::from(payload.into_boxed_slice()),
                    }));
                }
                return Err(AudioCaptureError::Timeout);
            }
            if wait.0 != WAIT_OBJECT_0 {
                return Err(AudioCaptureError::Backend {
                    details: format!("audio event wait failed with status {}", wait.0),
                });
            }
            if self
                .state
                .as_ref()
                .expect("state")
                .notification
                .take_pending()
            {
                self.state.take();
                self.format = None;
                return Ok(AudioCaptureEvent::DeviceLost);
            }
            match read_packet(self.state.as_mut().expect("state")) {
                Ok(Some(frame)) => {
                    if self
                        .state
                        .as_ref()
                        .expect("state")
                        .notification
                        .take_pending()
                    {
                        self.state.take();
                        self.format = None;
                        return Ok(AudioCaptureEvent::DeviceLost);
                    }
                    return Ok(AudioCaptureEvent::Frames(frame));
                }
                Ok(None) => continue,
                Err(AudioCaptureError::DeviceChanged) => {
                    self.state.take();
                    self.format = None;
                    return Ok(AudioCaptureEvent::DeviceLost);
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn stop(&mut self) -> Result<()> {
        self.state.take();
        self.format = None;
        Ok(())
    }

    fn format(&self) -> Option<AudioFormat> {
        self.format
    }
}

fn create_enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
        .map_err(|error| backend_error("create device enumerator", error))
}

fn default_device(enumerator: &IMMDeviceEnumerator, kind: WasapiCaptureKind) -> Result<IMMDevice> {
    unsafe {
        enumerator
            .GetDefaultAudioEndpoint(kind.flow(), eConsole)
            .map_err(|error| {
                if error.code() == windows::core::HRESULT::from_win32(ERROR_NOT_FOUND.0) {
                    AudioCaptureError::DeviceUnavailable
                } else {
                    backend_error("get default audio endpoint", error)
                }
            })
    }
}

fn select_device(
    enumerator: &IMMDeviceEnumerator,
    kind: WasapiCaptureKind,
    requested_id: Option<&str>,
) -> Result<IMMDevice> {
    match requested_id.filter(|id| !id.is_empty() && *id != "default") {
        Some(id) => {
            let wide = wide_null(id);
            unsafe { enumerator.GetDevice(PCWSTR(wide.as_ptr())) }
                .map_err(|_| AudioCaptureError::DeviceUnavailable)
        }
        None => default_device(enumerator, kind),
    }
}

const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};

fn device_friendly_name(device: &IMMDevice) -> Option<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let mut prop = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME).ok()?;
        let raw = prop.Anonymous.Anonymous.Anonymous.pwszVal;
        let name = if raw.is_null() {
            None
        } else {
            let len = (0..).take_while(|&i| *raw.0.add(i) != 0).count();
            let slice = std::slice::from_raw_parts(raw.0, len);
            let text = String::from_utf16_lossy(slice).trim().to_string();
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        };
        let _ = PropVariantClear(&mut prop);
        name
    }
}

fn device_id(device: &IMMDevice) -> Result<String> {
    let value = unsafe {
        device
            .GetId()
            .map_err(|error| backend_error("get audio endpoint id", error))?
    };
    let text =
        unsafe { value.to_string() }.map_err(|error| backend_error("read endpoint id", error))?;
    unsafe {
        CoTaskMemFree(Some(value.0.cast()));
    }
    Ok(text)
}

unsafe fn parse_format(format: &WAVEFORMATEX) -> Result<(AudioFormat, usize)> {
    let sample_format = match format.wBitsPerSample {
        16 => SampleFormat::S16,
        32 => SampleFormat::F32,
        bits => {
            return Err(AudioCaptureError::FormatUnsupported {
                reason: format!("WASAPI mix format uses {bits} bits per sample"),
            })
        }
    };
    if format.nSamplesPerSec == 0 || format.nChannels == 0 || format.nBlockAlign == 0 {
        return Err(AudioCaptureError::FormatUnsupported {
            reason: "WASAPI returned an incomplete mix format".to_string(),
        });
    }
    Ok((
        AudioFormat {
            sample_format,
            sample_rate: format.nSamplesPerSec,
            channels: format.nChannels,
        },
        format.nBlockAlign as usize,
    ))
}

fn read_packet(state: &mut CaptureState) -> Result<Option<AudioFrame>> {
    let packet_frames = unsafe { state.capture.GetNextPacketSize() }.map_err(map_capture_error)?;
    if packet_frames == 0 {
        return Ok(None);
    }

    let mut data = ptr::null_mut();
    let mut frames = 0_u32;
    let mut flags = 0_u32;
    let mut device_position = 0_u64;
    let mut qpc_position = QPC_POSITION_INVALID;
    let result = unsafe {
        state.capture.GetBuffer(
            &mut data,
            &mut frames,
            &mut flags,
            Some(&mut device_position),
            Some(&mut qpc_position),
        )
    };
    if let Err(error) = result {
        if error.code() == AUDCLNT_S_BUFFER_EMPTY {
            return Ok(None);
        }
        return Err(map_capture_error(error));
    }

    let byte_len = (frames as usize)
        .checked_mul(state.block_align)
        .ok_or_else(|| AudioCaptureError::Backend {
            details: "WASAPI packet size overflow".to_string(),
        })?;
    let payload = if flags & windows::Win32::Media::Audio::AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0
    {
        vec![0_u8; byte_len]
    } else if data.is_null() {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(data, byte_len).to_vec() }
    };
    let release_result = unsafe { state.capture.ReleaseBuffer(frames) };
    release_result.map_err(map_capture_error)?;
    if payload.len() != byte_len {
        return Err(AudioCaptureError::Backend {
            details: "WASAPI returned a null data pointer for a non-silent packet".to_string(),
        });
    }

    let pts = if qpc_position != QPC_POSITION_INVALID && qpc_position > 0 {
        qpc_position as i64
    } else {
        let mut qpc = 0_i64;
        let _ = unsafe { QueryPerformanceCounter(&mut qpc) };
        qpc
    };
    Ok(Some(AudioFrame {
        stream_id: media_types::StreamId(0),
        sample_format: state.format.sample_format,
        sample_rate: state.format.sample_rate,
        channels: state.format.channels,
        sample_count: frames,
        pts,
        time_base: QPC_TIME_BASE,
        data: Arc::from(payload.into_boxed_slice()),
    }))
}

fn map_capture_error(error: windows::core::Error) -> AudioCaptureError {
    if error.code() == AUDCLNT_E_DEVICE_INVALIDATED {
        AudioCaptureError::DeviceChanged
    } else {
        backend_error("read audio packet", error)
    }
}

fn backend_error(stage: &str, error: impl std::fmt::Display) -> AudioCaptureError {
    AudioCaptureError::Backend {
        details: format!("{stage}: {error}"),
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn pcwstr_matches(value: &PCWSTR, expected: &[u16]) -> bool {
    if value.0.is_null() || expected.is_empty() || expected.last() != Some(&0) {
        return false;
    }
    unsafe {
        expected
            .iter()
            .enumerate()
            .all(|(index, character)| *value.0.add(index) == *character)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave_format(bits: u16, channels: u16, sample_rate: u32, block_align: u16) -> WAVEFORMATEX {
        WAVEFORMATEX {
            wFormatTag: 1,
            nChannels: channels,
            nSamplesPerSec: sample_rate,
            nAvgBytesPerSec: sample_rate.saturating_mul(u32::from(block_align)),
            nBlockAlign: block_align,
            wBitsPerSample: bits,
            cbSize: 0,
        }
    }

    #[test]
    fn accepts_supported_s16_format() {
        let format = wave_format(16, 2, 48_000, 4);
        let parsed = unsafe { parse_format(&format) }.expect("supported format");
        assert_eq!(parsed.0.sample_format, SampleFormat::S16);
        assert_eq!(parsed.0.sample_rate, 48_000);
        assert_eq!(parsed.0.channels, 2);
        assert_eq!(parsed.1, 4);
    }

    #[test]
    fn accepts_supported_f32_format() {
        let format = wave_format(32, 1, 44_100, 4);
        let parsed = unsafe { parse_format(&format) }.expect("supported format");
        assert_eq!(parsed.0.sample_format, SampleFormat::F32);
        assert_eq!(parsed.0.sample_rate, 44_100);
        assert_eq!(parsed.0.channels, 1);
        assert_eq!(parsed.1, 4);
    }

    #[test]
    fn rejects_unsupported_or_incomplete_format() {
        let unsupported = wave_format(24, 2, 48_000, 6);
        assert!(matches!(
            unsafe { parse_format(&unsupported) },
            Err(AudioCaptureError::FormatUnsupported { .. })
        ));

        let incomplete = wave_format(16, 0, 48_000, 0);
        assert!(matches!(
            unsafe { parse_format(&incomplete) },
            Err(AudioCaptureError::FormatUnsupported { .. })
        ));
    }

    fn notification_client(
        kind: WasapiCaptureKind,
        follows_default: bool,
        selected_device_id: &str,
    ) -> (EndpointNotificationClient, Arc<NotificationState>) {
        let state = Arc::new(NotificationState::new(None));
        let client = EndpointNotificationClient::new(
            Arc::clone(&state),
            kind,
            follows_default,
            selected_device_id,
        );
        (client, state)
    }

    fn pcwstr(value: &str) -> (Vec<u16>, PCWSTR) {
        let value = wide_null(value);
        let pointer = PCWSTR(value.as_ptr());
        (value, pointer)
    }

    #[test]
    fn default_notification_only_invalidates_the_matching_flow_and_role() {
        let (client, state) = notification_client(WasapiCaptureKind::Loopback, true, "render-a");
        let (_new_id, new_id) = pcwstr("render-b");

        client.on_default_device_changed(eRender, eConsole, &new_id);
        assert!(state.take_pending());
        assert!(!state.take_pending());

        client.on_default_device_changed(eCapture, eConsole, &new_id);
        client.on_default_device_changed(
            eRender,
            windows::Win32::Media::Audio::eMultimedia,
            &new_id,
        );
        assert!(!state.take_pending());
    }

    #[test]
    fn explicit_endpoint_ignores_default_device_changes() {
        let (client, state) = notification_client(WasapiCaptureKind::Microphone, false, "mic-a");
        let (_new_id, new_id) = pcwstr("mic-b");

        client.on_default_device_changed(eCapture, eConsole, &new_id);
        assert!(!state.take_pending());
    }

    #[test]
    fn selected_endpoint_loss_is_latched_but_unrelated_changes_are_ignored() {
        let (client, state) = notification_client(WasapiCaptureKind::Microphone, false, "mic-a");
        let (_selected_id, selected_id) = pcwstr("mic-a");
        let (_other_id, other_id) = pcwstr("mic-b");

        client.on_device_state_changed(
            &other_id,
            windows::Win32::Media::Audio::DEVICE_STATE_DISABLED,
        );
        client.on_device_state_changed(&selected_id, DEVICE_STATE_ACTIVE);
        assert!(!state.take_pending());

        client.on_device_removed(&selected_id);
        assert!(state.take_pending());
    }
}
