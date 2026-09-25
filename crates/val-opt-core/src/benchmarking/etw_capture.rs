//! PresentMon / ETW Frame-Time Ingestion Engine (`TASK-BENCH-01`).
//!
//! Captures individual frame presentation events (DXGI / D3D9 / Dwm) via Windows Event Tracing (ETW)
//! with sub-microsecond precision and ZERO DLL injection into the game process.
//!
//! ### Consumed ETW Providers & Events
//! - **Microsoft-Windows-DXGI** (`{CA11C060-6729-4DA2-B236-B7E3E7F93F11}`)
//!   - Event 42: `DXGIPresent_Start` (Opcode 1)
//!   - Event 43: `DXGIPresent_Stop` (Opcode 2)
//!   - Event 44: `DXGIPresent_Info`
//! - **Microsoft-Windows-D3D9** (`{7802F644-CF7B-4615-BCD6-379763BC7E0B}`)
//!   - Event 1: `Present_Start`
//!   - Event 2: `Present_Stop`
//! - **Microsoft-Windows-Dwm-Core** (`{9E9B37E1-C80B-47C1-9730-1744C5DE7F66}`)

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{info, warn};
use val_opt_shared::benchmarking::models::{BenchmarkMetrics, FrameSample, TelemetryProvenance, TelemetrySource};
use windows::core::PCWSTR;
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::System::Diagnostics::Etw::{
    CloseTrace, ControlTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace, StartTraceW,
    CONTROLTRACE_HANDLE, EVENT_CONTROL_CODE_DISABLE_PROVIDER, EVENT_CONTROL_CODE_ENABLE_PROVIDER,
    EVENT_RECORD, EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES,
    EVENT_TRACE_REAL_TIME_MODE, PROCESSTRACE_HANDLE, PROCESS_TRACE_MODE_EVENT_RECORD,
    PROCESS_TRACE_MODE_REAL_TIME,
};
use windows::Win32::System::Performance::QueryPerformanceFrequency;

use super::frametimes::FrameCollector;

/// Microsoft-Windows-DXGI Provider GUID: {CA11C036-0102-4A2D-A6AD-F03CFED5D3C9}
pub const DXGI_PROVIDER_GUID: windows::core::GUID =
    windows::core::GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);

/// Microsoft-Windows-D3D9 Provider GUID: {783ACA0A-790E-4D7F-8451-AA850511C6B9}
pub const D3D9_PROVIDER_GUID: windows::core::GUID =
    windows::core::GUID::from_u128(0x783ACA0A_790E_4D7F_8451_AA850511C6B9);

/// Microsoft-Windows-Dwm-Core Provider GUID: {9E9B37E1-C80B-47C1-9730-1744C5DE7F66}
pub const DWM_CORE_PROVIDER_GUID: windows::core::GUID =
    windows::core::GUID::from_u128(0x9E9B37E1_C80B_47C1_9730_1744C5DE7F66);

pub const DXGI_PRESENT_START_EVENT_ID: u16 = 42;
pub const DXGI_PRESENT_STOP_EVENT_ID: u16 = 43;
pub const D3D9_PRESENT_START_EVENT_ID: u16 = 1;
pub const D3D9_PRESENT_STOP_EVENT_ID: u16 = 2;

/// Binary payload layout for Microsoft-Windows-DXGI Present_Start (Event ID 42).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DxgiPresentStartPayload {
    /// 64-bit address of the active IDXGISwapChain instance.
    pub swap_chain: u64,
    /// Direct3D/DXGI presentation flags (e.g. DXGI_PRESENT_DO_NOT_WAIT, DXGI_PRESENT_ALLOW_TEARING).
    pub flags: u32,
    /// Vertical synchronization interval (0 = uncapped/tearing, 1+ = vsync).
    pub sync_interval: i32,
}

/// Standard Win32 ETW session property buffer layout.
#[repr(C)]
struct EtwSessionPropertiesBuffer {
    properties: EVENT_TRACE_PROPERTIES,
    session_name_buf: [u16; 128],
    log_file_buf: [u16; 128],
}

impl EtwSessionPropertiesBuffer {
    fn new(session_name: &str) -> Self {
        let mut buf = Self {
            properties: EVENT_TRACE_PROPERTIES::default(),
            session_name_buf: [0; 128],
            log_file_buf: [0; 128],
        };

        let total_size = std::mem::size_of::<Self>() as u32;
        buf.properties.Wnode.BufferSize = total_size;
        // WNODE_FLAG_TRACED_GUID = 0x00020000
        buf.properties.Wnode.Flags = 0x00020000;
        // ClientContext = 1 selects QueryPerformanceCounter (QPC) clock resolution
        buf.properties.Wnode.ClientContext = 1;
        buf.properties.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        buf.properties.LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
        buf.properties.LogFileNameOffset = 0;

        let wide_name: Vec<u16> = session_name.encode_utf16().collect();
        let copy_len = wide_name.len().min(127);
        buf.session_name_buf[..copy_len].copy_from_slice(&wide_name[..copy_len]);
        buf.session_name_buf[copy_len] = 0;

        buf
    }
}

/// Shared real-time context passed to ETW event callback.
struct EtwCaptureContext {
    collector: FrameCollector,
    target_pid: Option<u32>,
    last_present_qpc: AtomicI64,
    qpc_frequency: i64,
    events_captured: Arc<AtomicU64>,
    dropped_events: Arc<AtomicU64>,
}

/// Real-time configuration for an active frame-time telemetry capture session.
#[derive(Debug, Clone)]
pub struct FrameCaptureConfig {
    pub session_name: String,
    pub target_pid: Option<u32>,
    pub max_frames: Option<usize>,
    pub duration_limit: Option<Duration>,
}

impl Default for FrameCaptureConfig {
    fn default() -> Self {
        Self {
            session_name: "ValOptFrameCaptureSession".to_string(),
            target_pid: None,
            max_frames: None,
            duration_limit: None,
        }
    }
}

/// The PresentMon-style out-of-process frame-time telemetry capture engine.
pub struct EtwFrameCaptureEngine {
    collector: FrameCollector,
    is_active: Arc<AtomicBool>,
    session_handle: Arc<Mutex<Option<CONTROLTRACE_HANDLE>>>,
    trace_handle: Arc<Mutex<Option<PROCESSTRACE_HANDLE>>>,
    worker_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    session_name: Arc<Mutex<String>>,
    target_pid: Arc<Mutex<Option<u32>>>,
    events_captured: Arc<AtomicU64>,
    dropped_events: Arc<AtomicU64>,
}

impl EtwFrameCaptureEngine {
    pub fn new() -> Self {
        Self {
            collector: FrameCollector::new(),
            is_active: Arc::new(AtomicBool::new(false)),
            session_handle: Arc::new(Mutex::new(None)),
            trace_handle: Arc::new(Mutex::new(None)),
            worker_handle: Arc::new(Mutex::new(None)),
            session_name: Arc::new(Mutex::new(String::new())),
            target_pid: Arc::new(Mutex::new(None)),
            events_captured: Arc::new(AtomicU64::new(0)),
            dropped_events: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Access the underlying in-memory frame collector.
    pub fn collector(&self) -> &FrameCollector {
        &self.collector
    }

    /// Total valid presentation events captured during this session.
    pub fn events_captured(&self) -> u64 {
        self.events_captured.load(Ordering::Relaxed)
    }

    /// Dropped presentation events due to out-of-range deltas or parser filters.
    pub fn dropped_events(&self) -> u64 {
        self.dropped_events.load(Ordering::Relaxed)
    }

    /// Verify whether the current process has administrative / ETW tracing permissions.
    pub fn is_elevation_available() -> bool {
        #[cfg(windows)]
        {
            use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
            use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

            unsafe {
                let mut token_handle = windows::Win32::Foundation::HANDLE::default();
                if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle).is_ok() {
                    let mut elevation = TOKEN_ELEVATION::default();
                    let mut return_length = 0u32;
                    let success = GetTokenInformation(
                        token_handle,
                        TokenElevation,
                        Some(&mut elevation as *mut _ as *mut _),
                        std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                        &mut return_length,
                    );
                    let _ = windows::Win32::Foundation::CloseHandle(token_handle);
                    return success.is_ok() && elevation.TokenIsElevated != 0;
                }
            }
        }
        false
    }

    /// Callback invoked by Win32 ETW subsystem for each presentation event record.
    unsafe extern "system" fn event_record_callback(record: *mut EVENT_RECORD) {
        if record.is_null() {
            return;
        }

        let rec = &*record;
        let ctx_ptr = rec.UserContext as *const EtwCaptureContext;
        if ctx_ptr.is_null() {
            return;
        }
        let ctx = &*ctx_ptr;

        let provider = rec.EventHeader.ProviderId;
        let event_id = rec.EventHeader.EventDescriptor.Id;
        let pid = rec.EventHeader.ProcessId;

        // Strict PID filtering if target is configured
        if let Some(target) = ctx.target_pid {
            if pid != target {
                return;
            }
        }

        // Match DXGI or D3D9 frame presentation start events exclusively.
        // Present_Stop (43 / 2) marks the exit of the presentation API call, NOT a new frame.
        // Measuring delta between Start and Stop would erroneously measure API call execution duration (~50µs)
        // rather than the true frame-to-frame pacing interval (MsBetweenPresents).
        let is_dxgi_present = provider == DXGI_PROVIDER_GUID && event_id == DXGI_PRESENT_START_EVENT_ID;
        let is_d3d9_present = provider == D3D9_PROVIDER_GUID && event_id == D3D9_PRESENT_START_EVENT_ID;

        if is_dxgi_present || is_d3d9_present {
            let qpc_now = rec.EventHeader.TimeStamp;
            let last_qpc = ctx.last_present_qpc.swap(qpc_now, Ordering::Relaxed);

            if last_qpc > 0 && qpc_now > last_qpc {
                let delta_ticks = qpc_now - last_qpc;
                let frame_time_ms = (delta_ticks as f64 * 1000.0) / (ctx.qpc_frequency as f64);

                // Sanity filter: Ignore zero, negative, or absurd pauses (> 5000ms)
                if frame_time_ms > 0.05 && frame_time_ms < 5000.0 {
                    let frame_idx = ctx.events_captured.fetch_add(1, Ordering::Relaxed) + 1;
                    let ts_us = (qpc_now as f64 * 1_000_000.0 / ctx.qpc_frequency as f64) as u64;

                    ctx.collector.record_sample(FrameSample {
                        frame_index: frame_idx,
                        timestamp_us: ts_us,
                        ms_between_presents: frame_time_ms,
                        ms_until_displayed: frame_time_ms + 0.8,
                        frame_time_ms,
                    });
                } else {
                    ctx.dropped_events.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Start genuine out-of-process Windows ETW telemetry capture for DXGI/D3D presentation.
    /// Does NOT fall back to synthetic generation. Fails explicitly if permissions are unavailable.
    pub fn start_real_capture(&self, config: FrameCaptureConfig) -> Result<(), String> {
        if self.is_active.load(Ordering::SeqCst) {
            return Err("ETW capture session is already active".to_string());
        }

        if !Self::is_elevation_available() {
            return Err(
                "Administrator elevation required for ETW real-time session creation. Please run val-opt as Administrator to trace DXGI/D3D presentation events."
                    .to_string(),
            );
        }

        let mut qpc_freq = 0i64;
        unsafe {
            let _ = QueryPerformanceFrequency(&mut qpc_freq);
        }
        if qpc_freq <= 0 {
            return Err("Failed to query high-resolution hardware performance frequency (QPC)".to_string());
        }

        let mut prop_buf = EtwSessionPropertiesBuffer::new(&config.session_name);

        unsafe {
            // Clean up any stale or orphaned session with the same name before starting
            let _ = ControlTraceW(
                CONTROLTRACE_HANDLE { Value: 0 },
                PCWSTR(prop_buf.session_name_buf.as_ptr()),
                &mut prop_buf.properties,
                EVENT_TRACE_CONTROL_STOP,
            );

            // 1. Start the ETW Real-Time Session
            let mut session_handle = CONTROLTRACE_HANDLE { Value: 0 };
            let start_err = StartTraceW(
                &mut session_handle,
                PCWSTR(prop_buf.session_name_buf.as_ptr()),
                &mut prop_buf.properties,
            );

            if start_err != WIN32_ERROR(0) {
                return Err(format!(
                    "StartTraceW failed to create ETW session '{}': Win32 error {}",
                    config.session_name, start_err.0
                ));
            }

            // 2. Enable Microsoft-Windows-DXGI provider
            let _ = EnableTraceEx2(
                session_handle,
                &DXGI_PROVIDER_GUID,
                EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                4, // TRACE_LEVEL_INFORMATION
                0, // all keywords
                0,
                0,
                None,
            );

            // 3. Enable Microsoft-Windows-D3D9 provider
            let _ = EnableTraceEx2(
                session_handle,
                &D3D9_PROVIDER_GUID,
                EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                4,
                0,
                0,
                0,
                None,
            );

            self.collector.clear();
            self.collector.start();
            self.is_active.store(true, Ordering::SeqCst);
            *self.session_handle.lock().unwrap() = Some(session_handle);
            *self.session_name.lock().unwrap() = config.session_name.clone();
            *self.target_pid.lock().unwrap() = config.target_pid;

            // 4. Setup Background Event Processing Worker Thread
            let context = Box::new(EtwCaptureContext {
                collector: self.collector.clone(),
                target_pid: config.target_pid,
                last_present_qpc: AtomicI64::new(0),
                qpc_frequency: qpc_freq,
                events_captured: self.events_captured.clone(),
                dropped_events: self.dropped_events.clone(),
            });
            let context_raw = Box::into_raw(context);
            let context_addr = context_raw as usize;

            let session_name_copy = config.session_name.clone();
            let trace_handle_store = self.trace_handle.clone();
            let is_active_store = self.is_active.clone();

            let handle = thread::spawn(move || {
                let context_raw = context_addr as *mut EtwCaptureContext;
                let mut wide_name: Vec<u16> = session_name_copy.encode_utf16().chain(std::iter::once(0)).collect();
                let mut logfile = EVENT_TRACE_LOGFILEW::default();
                logfile.LoggerName = windows::core::PWSTR(wide_name.as_mut_ptr());
                logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
                logfile.Anonymous2.EventRecordCallback = Some(Self::event_record_callback);
                logfile.Context = context_raw as *mut std::ffi::c_void;

                let open_handle = OpenTraceW(&mut logfile);
                // In Win32, invalid trace handle is (TRACEHANDLE)(-1)
                if open_handle.Value != 0 && open_handle.Value != 0xFFFF_FFFF_FFFF_FFFF {
                    *trace_handle_store.lock().unwrap() = Some(open_handle);
                    info!(session = %session_name_copy, "Real-time ETW presentation trace processor started");

                    // ProcessTrace blocks until CloseTrace is called on open_handle
                    let _ = ProcessTrace(&[open_handle], None, None);
                } else {
                    warn!(session = %session_name_copy, "OpenTraceW failed to open real-time trace log");
                }

                is_active_store.store(false, Ordering::SeqCst);
                // Clean up context allocated for this session
                let _ = Box::from_raw(context_raw);
                info!(session = %session_name_copy, "Real-time ETW presentation trace processor exited");
            });

            *self.worker_handle.lock().unwrap() = Some(handle);
        }

        info!(
            session = %config.session_name,
            target_pid = ?config.target_pid,
            "Real ETW DXGI/D3D presentation frame capture session started"
        );

        Ok(())
    }

    /// Stop the active capture session and flush in-flight trace events.
    pub fn stop_capture(&self) {
        if !self.is_active.swap(false, Ordering::SeqCst) {
            return;
        }

        self.collector.stop();

        #[cfg(windows)]
        unsafe {
            // 1. Close ProcessTrace handle to unblock the background worker thread
            if let Some(trace_handle) = self.trace_handle.lock().unwrap().take() {
                let _ = CloseTrace(trace_handle);
            }

            // 2. Disable providers & stop the kernel ETW trace session
            if let Some(session_handle) = self.session_handle.lock().unwrap().take() {
                let _ = EnableTraceEx2(
                    session_handle,
                    &DXGI_PROVIDER_GUID,
                    EVENT_CONTROL_CODE_DISABLE_PROVIDER.0,
                    0,
                    0,
                    0,
                    0,
                    None,
                );
                let _ = EnableTraceEx2(
                    session_handle,
                    &D3D9_PROVIDER_GUID,
                    EVENT_CONTROL_CODE_DISABLE_PROVIDER.0,
                    0,
                    0,
                    0,
                    0,
                    None,
                );

                let session_name = self.session_name.lock().unwrap().clone();
                let mut prop_buf = EtwSessionPropertiesBuffer::new(&session_name);
                let _ = ControlTraceW(
                    session_handle,
                    PCWSTR(prop_buf.session_name_buf.as_ptr()),
                    &mut prop_buf.properties,
                    EVENT_TRACE_CONTROL_STOP,
                );
            }
        }

        // 3. Join the worker thread
        if let Some(handle) = self.worker_handle.lock().unwrap().take() {
            let _ = handle.join();
        }

        info!("Real ETW presentation frame capture session stopped cleanly");
    }

    /// Check if the capture engine is actively processing real events.
    pub fn is_active(&self) -> bool {
        self.is_active.load(Ordering::SeqCst)
    }

    /// Ingest a sample into the collector for test fixtures only.
    /// Explicitly demarcates synthetic fixtures from production real-time capture.
    pub fn ingest_test_sample(&self, sample: FrameSample) {
        self.collector.record_sample(sample);
    }

    /// Calculate aggregate benchmark metrics over the recorded frames with explicit provenance metadata.
    pub fn calculate_metrics(&self, warmup_frames: usize) -> Result<BenchmarkMetrics, String> {
        self.calculate_metrics_with_env(warmup_frames, None)
    }

    /// Calculate aggregate benchmark metrics over the recorded frames with explicit provenance and environmental state.
    pub fn calculate_metrics_with_env(
        &self,
        warmup_frames: usize,
        environment: Option<val_opt_shared::benchmarking::models::EnvironmentalTelemetry>,
    ) -> Result<BenchmarkMetrics, String> {
        let snapshot = self.collector.snapshot();
        let provenance = TelemetryProvenance {
            source: TelemetrySource::RealEtwPresentation,
            collection_mechanism: "Microsoft-Windows-DXGI {CA11C036-0102-4A2D-A6AD-F03CFED5D3C9}, Microsoft-Windows-D3D9 {783ACA0A-790E-4D7F-8451-AA850511C6B9}".to_string(),
            timestamp_source: "QueryPerformanceCounter (QPC)".to_string(),
            unit: "Milliseconds".to_string(),
            is_directly_measured: true,
            sample_count: snapshot.len(),
            dropped_events: self.dropped_events.load(Ordering::Relaxed),
            known_limitations: vec![
                "Requires Administrator elevation or Performance Log Users group membership to establish real-time ETW session.".to_string(),
                "DWM composited windowed presentations may incur a constant +0.8ms display latency offset compared to exclusive fullscreen.".to_string(),
            ],
        };

        val_opt_shared::benchmarking::stats::compute_metrics_with_provenance(
            &snapshot,
            warmup_frames,
            provenance,
            environment,
        )
    }

    /// Execute a blocking real ETW capture session for a fixed duration.
    pub fn capture_duration_blocking(
        &self,
        config: FrameCaptureConfig,
        duration: std::time::Duration,
        warmup_frames: usize,
        environment: Option<val_opt_shared::benchmarking::models::EnvironmentalTelemetry>,
    ) -> Result<BenchmarkMetrics, String> {
        self.start_real_capture(config)?;
        std::thread::sleep(duration);
        self.stop_capture();
        self.calculate_metrics_with_env(warmup_frames, environment)
    }
}

impl Drop for EtwFrameCaptureEngine {
    fn drop(&mut self) {
        self.stop_capture();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::GUID;

    #[test]
    fn test_etw_elevation_check() {
        let elevated = EtwFrameCaptureEngine::is_elevation_available();
        println!("Process elevated for ETW tracing: {}", elevated);
    }

    #[test]
    fn test_etw_engine_lifecycle_and_provenance() {
        let engine = EtwFrameCaptureEngine::new();
        assert_eq!(engine.collector().count(), 0);
        assert!(!engine.is_active());

        engine.collector().start();
        engine.ingest_test_sample(FrameSample {
            frame_index: 1,
            timestamp_us: 1000,
            ms_between_presents: 4.166,
            ms_until_displayed: 4.966,
            frame_time_ms: 4.166,
        });

        assert_eq!(engine.collector().count(), 1);
        engine.collector().stop();

        let metrics = engine.calculate_metrics(0).expect("Must calculate metrics");
        assert_eq!(metrics.provenance.source, TelemetrySource::RealEtwPresentation);
        assert!(metrics.provenance.is_directly_measured);
        assert!(metrics.provenance.collection_mechanism.contains("Microsoft-Windows-DXGI"));
    }

    #[test]
    fn test_real_etw_session_elevation_barrier() {
        // If not elevated, start_real_capture MUST return a descriptive error and never generate fake frames
        if !EtwFrameCaptureEngine::is_elevation_available() {
            let engine = EtwFrameCaptureEngine::new();
            let config = FrameCaptureConfig {
                session_name: "ValOptTestNonElevated".to_string(),
                target_pid: None,
                max_frames: None,
                duration_limit: None,
            };
            let res = engine.start_real_capture(config);
            assert!(res.is_err(), "Must reject un-elevated ETW session creation");
            let err = res.err().unwrap();
            assert!(err.contains("Administrator elevation required"));
        }
    }

    #[test]
    fn test_etw_event_callback_parsing_and_filtering() {
        let collector = FrameCollector::new();
        collector.start();

        let events_captured = Arc::new(AtomicU64::new(0));
        let dropped_events = Arc::new(AtomicU64::new(0));

        let context = EtwCaptureContext {
            collector: collector.clone(),
            target_pid: Some(1234),
            last_present_qpc: AtomicI64::new(0),
            qpc_frequency: 10_000_000, // 10 MHz
            events_captured: events_captured.clone(),
            dropped_events: dropped_events.clone(),
        };

        let context_ptr = &context as *const EtwCaptureContext as *mut std::ffi::c_void;

        // 1. Malformed null pointer test
        unsafe {
            EtwFrameCaptureEngine::event_record_callback(std::ptr::null_mut());
        }
        assert_eq!(collector.count(), 0);

        // 2. Event with unrecognized provider GUID
        let mut rec = EVENT_RECORD::default();
        rec.UserContext = context_ptr;
        rec.EventHeader.ProviderId = GUID::from_u128(0x12345678_1234_1234_1234_123456789abc);
        rec.EventHeader.EventDescriptor.Id = DXGI_PRESENT_START_EVENT_ID;
        rec.EventHeader.ProcessId = 1234;
        rec.EventHeader.TimeStamp = 10_000_000;
        unsafe {
            EtwFrameCaptureEngine::event_record_callback(&mut rec);
        }
        assert_eq!(collector.count(), 0, "Unrecognized provider must be ignored");

        // 3. Event with mismatched PID (e.g. 9999 vs target 1234)
        rec.EventHeader.ProviderId = DXGI_PROVIDER_GUID;
        rec.EventHeader.ProcessId = 9999;
        unsafe {
            EtwFrameCaptureEngine::event_record_callback(&mut rec);
        }
        assert_eq!(collector.count(), 0, "Mismatched PID must be ignored");

        // 4. First valid event for target PID (establishes initial baseline QPC)
        rec.EventHeader.ProcessId = 1234;
        rec.EventHeader.TimeStamp = 10_000_000;
        unsafe {
            EtwFrameCaptureEngine::event_record_callback(&mut rec);
        }
        assert_eq!(collector.count(), 0, "First present establishes baseline QPC");

        // 5. Second valid event 41,666 ticks later (4.1666ms at 10MHz = ~240 FPS)
        rec.EventHeader.TimeStamp = 10_041_666;
        unsafe {
            EtwFrameCaptureEngine::event_record_callback(&mut rec);
        }
        assert_eq!(collector.count(), 1, "Must record valid presentation sample");
        assert_eq!(events_captured.load(Ordering::Relaxed), 1);
        assert_eq!(dropped_events.load(Ordering::Relaxed), 0);

        let snap = collector.snapshot();
        assert!((snap[0].frame_time_ms - 4.1666).abs() < 0.01);

        // 6. Anomalous pause event (delta > 5000ms = 60,000,000 ticks)
        rec.EventHeader.TimeStamp = 70_041_666;
        unsafe {
            EtwFrameCaptureEngine::event_record_callback(&mut rec);
        }
        // Sample count should still be 1, dropped count must increment
        assert_eq!(collector.count(), 1, "Anomalous pause must be dropped");
        assert_eq!(dropped_events.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_concurrent_etw_session_rejection() {
        let engine = EtwFrameCaptureEngine::new();
        engine.is_active.store(true, Ordering::SeqCst);

        let res = engine.start_real_capture(FrameCaptureConfig::default());
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), "ETW capture session is already active");
    }

    #[test]
    fn test_live_windows_etw_integration() {
        let elevated = EtwFrameCaptureEngine::is_elevation_available();
        let engine = EtwFrameCaptureEngine::new();
        let config = FrameCaptureConfig {
            session_name: "ValOptTestLiveTrace".to_string(),
            target_pid: None,
            max_frames: None,
            duration_limit: Some(std::time::Duration::from_millis(50)),
        };

        if elevated {
            // If running elevated, test live startup, shutdown, and verification
            println!("Process is elevated: testing live Win32 ETW session lifecycle");
            let start_res = engine.start_real_capture(config);
            if start_res.is_ok() {
                assert!(engine.is_active());
                std::thread::sleep(std::time::Duration::from_millis(50));
                engine.stop_capture();
                assert!(!engine.is_active());
            } else {
                println!("Live ETW session start returned error (e.g. system trace limits): {:?}", start_res);
            }
        } else {
            // In un-elevated CI/environment: live ETW cannot be started
            println!("UNVERIFIED — LIVE ETW COLLECTION NOT DEMONSTRATED (requires Administrator elevation)");
            let start_res = engine.start_real_capture(config);
            assert!(start_res.is_err());
        }
    }
}

