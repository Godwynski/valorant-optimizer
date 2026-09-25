//! Kernel ETW DPC / ISR Event Session Manager.
//!
//! Controls a real-time Windows ETW kernel trace session targeting:
//! - EVENT_TRACE_FLAG_DPC (0x00000020)
//! - EVENT_TRACE_FLAG_INTERRUPT (0x00000040)
//!
//! Streams events in real-time from the Windows NT Kernel Logger without disk buffering,
//! parses classic MOF PerfInfo DPC/ISR event records with QPC timestamps, measures DPC/ISR
//! durations with microsecond precision, and correlates them with physical driver modules
//! via KernelDriverIsolator.
//!
//! Distinguishes genuine kernel ETW telemetry from offline synthetic test fixtures.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{info, warn};
use val_opt_shared::benchmarking::models::{TelemetryProvenance, TelemetrySource};
use val_opt_shared::models::latency::{DpcIsrSample, LatencyMeasurementStatus};

#[cfg(windows)]
use windows::core::{GUID, PWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::WIN32_ERROR;
#[cfg(windows)]
use windows::Win32::System::Diagnostics::Etw::{
    CloseTrace, ControlTraceW, OpenTraceW, ProcessTrace, StartTraceW,
    CONTROLTRACE_HANDLE, EVENT_RECORD, EVENT_TRACE_CONTROL_STOP,
    EVENT_TRACE_FLAG_DPC, EVENT_TRACE_FLAG_INTERRUPT, EVENT_TRACE_LOGFILEW,
    EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE, KERNEL_LOGGER_NAMEW,
    PROCESSTRACE_HANDLE, PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_REAL_TIME,
};
#[cfg(windows)]
use windows::Win32::System::Performance::QueryPerformanceFrequency;

use super::driver_isolator::KernelDriverIsolator;
use super::monitor::LatencyMonitor;

/// PerfInfo Guid: {ce1db8ac-30e0-47de-8733-86f68e0d63a0}
#[cfg(windows)]
const PERFINFO_GUID: GUID = GUID::from_u128(0xce1db8ac_30e0_47de_8733_86f68e0d63a0);

/// SystemTraceControlGuid: {9e814aad-3204-11d2-9a82-0060081841cd}
#[cfg(windows)]
const SYSTEM_TRACE_CONTROL_GUID: GUID = GUID::from_u128(0x9e814aad_3204_11d2_9a82_0060081841cd);

/// Configuration for the Kernel Latency ETW Session.
#[derive(Debug, Clone)]
pub struct LatencySessionConfig {
    pub buffer_size_kb: u32,
    pub min_buffers: u32,
    pub max_buffers: u32,
}

impl Default for LatencySessionConfig {
    fn default() -> Self {
        Self {
            buffer_size_kb: 64,
            min_buffers: 4,
            max_buffers: 32,
        }
    }
}

/// Contiguous Win32 buffer holding EVENT_TRACE_PROPERTIES followed by wide session name strings.
#[cfg(windows)]
#[repr(C)]
struct KernelTracePropertiesBuffer {
    properties: EVENT_TRACE_PROPERTIES,
    session_name_buf: [u16; 64],
    log_file_buf: [u16; 64],
}

#[cfg(windows)]
impl KernelTracePropertiesBuffer {
    fn new(config: &LatencySessionConfig) -> Self {
        let mut buf = Self {
            properties: EVENT_TRACE_PROPERTIES::default(),
            session_name_buf: [0; 64],
            log_file_buf: [0; 64],
        };

        let total_size = std::mem::size_of::<Self>() as u32;
        buf.properties.Wnode.BufferSize = total_size;
        // WNODE_FLAG_TRACED_GUID = 0x00020000
        buf.properties.Wnode.Flags = 0x00020000;
        buf.properties.Wnode.Guid = SYSTEM_TRACE_CONTROL_GUID;
        // ClientContext = 1 selects QueryPerformanceCounter (QPC) clock resolution
        buf.properties.Wnode.ClientContext = 1;
        buf.properties.BufferSize = config.buffer_size_kb;
        buf.properties.MinimumBuffers = config.min_buffers;
        buf.properties.MaximumBuffers = config.max_buffers;
        buf.properties.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        buf.properties.EnableFlags = EVENT_TRACE_FLAG_DPC | EVENT_TRACE_FLAG_INTERRUPT;
        buf.properties.LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
        buf.properties.LogFileNameOffset = 0;

        let kernel_name: Vec<u16> = "NT Kernel Logger\0".encode_utf16().collect();
        let copy_len = kernel_name.len().min(63);
        buf.session_name_buf[..copy_len].copy_from_slice(&kernel_name[..copy_len]);
        buf.session_name_buf[copy_len] = 0;

        buf
    }
}

/// Shared real-time context passed to Win32 kernel event record callback.
struct KernelSessionContext {
    isolator: Arc<KernelDriverIsolator>,
    monitor: Arc<LatencyMonitor>,
    total_dpcs: Arc<AtomicU64>,
    total_isrs: Arc<AtomicU64>,
    max_dpc_us: Arc<AtomicU64>,
    max_isr_us: Arc<AtomicU64>,
    qpc_frequency: i64,
}

impl KernelSessionContext {
    fn ingest_sample(&self, timestamp_us: u64, duration_us: u64, routine_address: u64, is_dpc: bool) {
        if is_dpc {
            self.total_dpcs.fetch_add(1, Ordering::Relaxed);
            let mut current_max = self.max_dpc_us.load(Ordering::Relaxed);
            while duration_us > current_max {
                match self.max_dpc_us.compare_exchange_weak(
                    current_max,
                    duration_us,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(actual) => current_max = actual,
                }
            }
        } else {
            self.total_isrs.fetch_add(1, Ordering::Relaxed);
            let mut current_max = self.max_isr_us.load(Ordering::Relaxed);
            while duration_us > current_max {
                match self.max_isr_us.compare_exchange_weak(
                    current_max,
                    duration_us,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(actual) => current_max = actual,
                }
            }
        }

        // 1. Resolve routine address to loaded kernel driver
        let driver_name = self.isolator.record_execution(routine_address, duration_us);

        // 2. Feed into ring buffer monitor & trigger threshold warnings
        let sample = DpcIsrSample {
            timestamp_us,
            duration_us,
            routine_address,
            is_dpc,
            driver_name: Some(driver_name),
        };
        self.monitor.ingest_sample(sample);
    }
}

/// The unified manager orchestrating real-time kernel ETW DPC/ISR tracing and driver isolation.
pub struct KernelLatencySessionManager {
    isolator: Arc<KernelDriverIsolator>,
    monitor: Arc<LatencyMonitor>,
    is_running: Arc<AtomicBool>,
    #[cfg(windows)]
    session_handle: Arc<Mutex<Option<CONTROLTRACE_HANDLE>>>,
    #[cfg(windows)]
    trace_handle: Arc<Mutex<Option<PROCESSTRACE_HANDLE>>>,
    worker_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    total_dpcs: Arc<AtomicU64>,
    total_isrs: Arc<AtomicU64>,
    max_dpc_us: Arc<AtomicU64>,
    max_isr_us: Arc<AtomicU64>,
    measurement_status: Arc<RwLock<LatencyMeasurementStatus>>,
    status_detail: Arc<RwLock<Option<String>>>,
}

impl KernelLatencySessionManager {
    pub fn new() -> Self {
        Self {
            isolator: Arc::new(KernelDriverIsolator::new()),
            monitor: Arc::new(LatencyMonitor::default()),
            is_running: Arc::new(AtomicBool::new(false)),
            #[cfg(windows)]
            session_handle: Arc::new(Mutex::new(None)),
            #[cfg(windows)]
            trace_handle: Arc::new(Mutex::new(None)),
            worker_handle: Arc::new(Mutex::new(None)),
            total_dpcs: Arc::new(AtomicU64::new(0)),
            total_isrs: Arc::new(AtomicU64::new(0)),
            max_dpc_us: Arc::new(AtomicU64::new(0)),
            max_isr_us: Arc::new(AtomicU64::new(0)),
            measurement_status: Arc::new(RwLock::new(LatencyMeasurementStatus::SessionError)),
            status_detail: Arc::new(RwLock::new(None)),
        }
    }

    /// Access the driver fault isolator.
    pub fn isolator(&self) -> &Arc<KernelDriverIsolator> {
        &self.isolator
    }

    /// Access the real-time latency health monitor.
    pub fn monitor(&self) -> &Arc<LatencyMonitor> {
        &self.monitor
    }

    /// Check whether the current process has administrative elevation for kernel ETW.
    pub fn is_elevation_available() -> bool {
        crate::benchmarking::EtwFrameCaptureEngine::is_elevation_available()
    }

    /// Current measurement status.
    pub fn measurement_status(&self) -> LatencyMeasurementStatus {
        *self.measurement_status.read().unwrap()
    }

    /// Current detailed status or error message.
    pub fn status_detail(&self) -> Option<String> {
        self.status_detail.read().unwrap().clone()
    }

    /// Provenance metadata documenting telemetry source and collection characteristics.
    pub fn provenance(&self) -> TelemetryProvenance {
        let status = self.measurement_status();
        let (source, mechanism, derived, limitations) = match status {
            LatencyMeasurementStatus::RealEtwCollected => (
                TelemetrySource::RealKernelDpcIsr,
                "Windows ETW (NT Kernel Logger: EVENT_TRACE_FLAG_DPC | EVENT_TRACE_FLAG_INTERRUPT, PerfInfo GUID {ce1db8ac-30e0-47de-8733-86f68e0d63a0})",
                true,
                "Kernel ASLR (KASLR) may obscure routine names if driver query is unprivileged; system-wide kernel trace allows only one active NT Kernel Logger"
            ),
            LatencyMeasurementStatus::UnsupportedElevationRequired => (
                TelemetrySource::RealKernelDpcIsr,
                "UNSUPPORTED — Administrator elevation required to access NT Kernel Logger",
                false,
                "Process is not elevated; kernel ETW telemetry could not be collected"
            ),
            LatencyMeasurementStatus::UnsupportedSessionInUse => (
                TelemetrySource::RealKernelDpcIsr,
                "UNSUPPORTED — NT Kernel Logger session locked by another tool (e.g. WPR)",
                false,
                "Concurrent kernel trace session active on host system"
            ),
            LatencyMeasurementStatus::SessionError => (
                TelemetrySource::RealKernelDpcIsr,
                "SESSION_ERROR — Win32 ETW session initialization failed",
                false,
                "ETW subsystem returned failure during trace setup"
            ),
            LatencyMeasurementStatus::SyntheticTestFixture => (
                TelemetrySource::SyntheticFixture,
                "SYNTHETIC TEST FIXTURE (Offline QA / Unit Test Harness)",
                false,
                "Data is generated in-memory for testing and does not represent real kernel hardware performance"
            ),
        };

        let sample_count = (self.total_dpcs() + self.total_isrs()) as usize;
        TelemetryProvenance {
            source,
            collection_mechanism: mechanism.to_string(),
            timestamp_source: "Win32 QueryPerformanceCounter (QPC)".to_string(),
            unit: "Microseconds (µs)".to_string(),
            is_directly_measured: derived,
            sample_count,
            dropped_events: 0,
            known_limitations: vec![limitations.to_string()],
        }
    }

    /// Callback invoked by Win32 ETW subsystem for each kernel event record.
    #[cfg(windows)]
    unsafe extern "system" fn kernel_event_record_callback(record: *mut EVENT_RECORD) {
        if record.is_null() {
            return;
        }

        let rec = &*record;
        let ctx_ptr = rec.UserContext as *const KernelSessionContext;
        if ctx_ptr.is_null() {
            return;
        }
        let ctx = &*ctx_ptr;

        let provider = rec.EventHeader.ProviderId;
        if provider != PERFINFO_GUID && provider != SYSTEM_TRACE_CONTROL_GUID {
            return;
        }

        let opcode = rec.EventHeader.EventDescriptor.Opcode;
        let is_dpc = opcode == 66 || opcode == 68 || opcode == 69;
        let is_isr = opcode == 67;

        if !is_dpc && !is_isr {
            return;
        }

        if rec.UserDataLength < 16 || rec.UserData.is_null() {
            return;
        }

        let initial_time = *(rec.UserData as *const u64);
        let routine = *((rec.UserData as usize + 8) as *const u64);
        let event_time = rec.EventHeader.TimeStamp as u64;

        if routine == 0 {
            return;
        }

        let duration_ticks = if event_time > initial_time {
            event_time - initial_time
        } else {
            0
        };

        let duration_us = if ctx.qpc_frequency > 0 {
            (duration_ticks * 1_000_000) / (ctx.qpc_frequency as u64)
        } else {
            duration_ticks
        };

        let timestamp_us = if ctx.qpc_frequency > 0 {
            (event_time * 1_000_000) / (ctx.qpc_frequency as u64)
        } else {
            event_time
        };

        // Filter anomalous durations (>10s indicates lost event pairing or corrupted packet)
        if duration_us < 10_000_000 {
            ctx.ingest_sample(timestamp_us, duration_us, routine, is_dpc);
        }
    }

    /// Ingest an individual DPC or ISR sample into the session pipelines (used by tests & manual feeds).
    pub fn ingest_sample(&self, timestamp_us: u64, duration_us: u64, routine_address: u64, is_dpc: bool) {
        if is_dpc {
            self.total_dpcs.fetch_add(1, Ordering::Relaxed);
            let mut current_max = self.max_dpc_us.load(Ordering::Relaxed);
            while duration_us > current_max {
                match self.max_dpc_us.compare_exchange_weak(
                    current_max,
                    duration_us,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(actual) => current_max = actual,
                }
            }
        } else {
            self.total_isrs.fetch_add(1, Ordering::Relaxed);
            let mut current_max = self.max_isr_us.load(Ordering::Relaxed);
            while duration_us > current_max {
                match self.max_isr_us.compare_exchange_weak(
                    current_max,
                    duration_us,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(actual) => current_max = actual,
                }
            }
        }

        let driver_name = self.isolator.record_execution(routine_address, duration_us);
        let sample = DpcIsrSample {
            timestamp_us,
            duration_us,
            routine_address,
            is_dpc,
            driver_name: Some(driver_name),
        };
        self.monitor.ingest_sample(sample);
    }

    /// Start genuine Windows NT Kernel Logger ETW tracing session.
    /// Does NOT fall back to synthetic data. Fails explicitly if elevation or provider is unavailable.
    pub fn start_session(&mut self, config: LatencySessionConfig) -> Result<(), String> {
        if self.is_running.load(Ordering::SeqCst) {
            return Err("Kernel latency ETW session is already active".to_string());
        }

        // Elevation barrier: Windows NT Kernel Logger requires full Administrator privileges
        if !Self::is_elevation_available() {
            *self.measurement_status.write().unwrap() = LatencyMeasurementStatus::UnsupportedElevationRequired;
            let err = "Administrator elevation required for NT Kernel Logger DPC/ISR ETW tracing.".to_string();
            *self.status_detail.write().unwrap() = Some(err.clone());
            return Err(err);
        }

        #[cfg(windows)]
        {
            let mut qpc_freq = 0i64;
            unsafe {
                if QueryPerformanceFrequency(&mut qpc_freq).is_err() || qpc_freq <= 0 {
                    qpc_freq = 10_000_000;
                }
            }

            let mut prop_buf = KernelTracePropertiesBuffer::new(&config);

            unsafe {
                // Clear any stale orphaned kernel session from previous runs
                let _ = ControlTraceW(
                    CONTROLTRACE_HANDLE { Value: 0 },
                    KERNEL_LOGGER_NAMEW,
                    &mut prop_buf.properties,
                    EVENT_TRACE_CONTROL_STOP,
                );

                let mut session_handle = CONTROLTRACE_HANDLE { Value: 0 };
                let start_err = StartTraceW(
                    &mut session_handle,
                    KERNEL_LOGGER_NAMEW,
                    &mut prop_buf.properties,
                );

                if start_err != WIN32_ERROR(0) {
                    let err_code = start_err.0;
                    if err_code == 183 {
                        // ERROR_ALREADY_EXISTS
                        *self.measurement_status.write().unwrap() = LatencyMeasurementStatus::UnsupportedSessionInUse;
                        let err = "NT Kernel Logger is in use by another tracing session (e.g. WPR or performance analyzer).".to_string();
                        *self.status_detail.write().unwrap() = Some(err.clone());
                        return Err(err);
                    } else if err_code == 5 {
                        // ERROR_ACCESS_DENIED
                        *self.measurement_status.write().unwrap() = LatencyMeasurementStatus::UnsupportedElevationRequired;
                        let err = "Access denied: Administrator privileges required for NT Kernel Logger.".to_string();
                        *self.status_detail.write().unwrap() = Some(err.clone());
                        return Err(err);
                    } else {
                        *self.measurement_status.write().unwrap() = LatencyMeasurementStatus::SessionError;
                        let err = format!("StartTraceW failed to create NT Kernel Logger session: Win32 error 0x{:X}", err_code);
                        *self.status_detail.write().unwrap() = Some(err.clone());
                        return Err(err);
                    }
                }

                self.isolator.clear_stats();
                self.monitor.clear();
                self.total_dpcs.store(0, Ordering::SeqCst);
                self.total_isrs.store(0, Ordering::SeqCst);
                self.max_dpc_us.store(0, Ordering::SeqCst);
                self.max_isr_us.store(0, Ordering::SeqCst);
                self.is_running.store(true, Ordering::SeqCst);
                *self.session_handle.lock().unwrap() = Some(session_handle);
                *self.measurement_status.write().unwrap() = LatencyMeasurementStatus::RealEtwCollected;
                *self.status_detail.write().unwrap() = None;

                // Setup Background Event Processing Worker Thread
                let context = Box::new(KernelSessionContext {
                    isolator: self.isolator.clone(),
                    monitor: self.monitor.clone(),
                    total_dpcs: self.total_dpcs.clone(),
                    total_isrs: self.total_isrs.clone(),
                    max_dpc_us: self.max_dpc_us.clone(),
                    max_isr_us: self.max_isr_us.clone(),
                    qpc_frequency: qpc_freq,
                });
                let context_raw = Box::into_raw(context);
                let context_addr = context_raw as usize;

                let trace_handle_store = self.trace_handle.clone();
                let is_running_store = self.is_running.clone();

                let handle = thread::spawn(move || {
                    let context_raw = context_addr as *mut KernelSessionContext;
                    let mut kernel_logger_w: Vec<u16> = "NT Kernel Logger\0".encode_utf16().collect();
                    let mut logfile = EVENT_TRACE_LOGFILEW::default();
                    logfile.LoggerName = PWSTR(kernel_logger_w.as_mut_ptr());
                    logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
                    logfile.Anonymous2.EventRecordCallback = Some(Self::kernel_event_record_callback);
                    logfile.Context = context_raw as *mut std::ffi::c_void;

                    let open_handle = OpenTraceW(&mut logfile);
                    if open_handle.Value != 0 && open_handle.Value != 0xFFFF_FFFF_FFFF_FFFF {
                        *trace_handle_store.lock().unwrap() = Some(open_handle);
                        info!("Real-time NT Kernel Logger DPC/ISR trace processor started");

                        // ProcessTrace blocks until CloseTrace is called
                        let _ = ProcessTrace(&[open_handle], None, None);
                    } else {
                        warn!("OpenTraceW failed to open NT Kernel Logger real-time log");
                    }

                    is_running_store.store(false, Ordering::SeqCst);
                    let _ = Box::from_raw(context_raw);
                    info!("Real-time NT Kernel Logger DPC/ISR trace processor exited");
                });

                *self.worker_handle.lock().unwrap() = Some(handle);
            }

            info!("Kernel latency ETW monitoring session started successfully");
            Ok(())
        }

        #[cfg(not(windows))]
        {
            *self.measurement_status.write().unwrap() = LatencyMeasurementStatus::SessionError;
            Err("Windows ETW kernel tracing is only supported on Windows operating systems".to_string())
        }
    }

    /// Stop the active latency monitoring session.
    pub fn stop_session(&mut self) {
        if self.is_running.swap(false, Ordering::SeqCst) {
            #[cfg(windows)]
            unsafe {
                if let Some(trace_h) = self.trace_handle.lock().unwrap().take() {
                    let _ = CloseTrace(trace_h);
                }

                if let Some(session_h) = self.session_handle.lock().unwrap().take() {
                    let mut prop_buf = KernelTracePropertiesBuffer::new(&LatencySessionConfig::default());
                    let _ = ControlTraceW(
                        session_h,
                        KERNEL_LOGGER_NAMEW,
                        &mut prop_buf.properties,
                        EVENT_TRACE_CONTROL_STOP,
                    );
                }
            }

            if let Some(handle) = self.worker_handle.lock().unwrap().take() {
                let _ = handle.join();
            }

            info!("Kernel latency ETW monitoring session stopped");
        }
    }

    /// Run real Windows ETW kernel session for the given duration.
    /// Does NOT fall back to synthetic data. Returns error if session could not be started.
    pub fn run_real_session(&mut self, duration: Duration) -> Result<(), String> {
        self.start_session(LatencySessionConfig::default())?;
        thread::sleep(duration);
        self.stop_session();
        Ok(())
    }

    /// Check if the session is currently active.
    pub fn is_active(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    /// Get total DPCs captured during the session.
    pub fn total_dpcs(&self) -> u64 {
        self.total_dpcs.load(Ordering::Relaxed)
    }

    /// Get total ISRs captured during the session.
    pub fn total_isrs(&self) -> u64 {
        self.total_isrs.load(Ordering::Relaxed)
    }

    /// Get highest single DPC execution time in microseconds.
    pub fn max_dpc_us(&self) -> u64 {
        self.max_dpc_us.load(Ordering::Relaxed)
    }

    /// Get highest single ISR execution time in microseconds.
    pub fn max_isr_us(&self) -> u64 {
        self.max_isr_us.load(Ordering::Relaxed)
    }

    /// Run synthetic workload session (strictly isolated for offline unit tests and automated QA).
    /// Explicitly flags measurement_status as SyntheticTestFixture.
    pub fn run_synthetic_session(&mut self, duration: Duration, inject_spike: bool) {
        if self.is_running.load(Ordering::SeqCst) {
            return;
        }

        self.is_running.store(true, Ordering::SeqCst);
        self.isolator.clear_stats();
        self.monitor.clear();
        self.total_dpcs.store(0, Ordering::SeqCst);
        self.total_isrs.store(0, Ordering::SeqCst);
        self.max_dpc_us.store(0, Ordering::SeqCst);
        self.max_isr_us.store(0, Ordering::SeqCst);
        *self.measurement_status.write().unwrap() = LatencyMeasurementStatus::SyntheticTestFixture;
        *self.status_detail.write().unwrap() = Some("Synthetic test fixture executed for offline QA".to_string());

        let isolator = self.isolator.clone();
        let drivers = isolator.get_top_drivers(10);
        let default_driver_addr = 0xFFFFF80001000000;

        let start = std::time::Instant::now();
        let mut sample_count = 0u64;

        while start.elapsed() < duration {
            sample_count += 1;
            let ts = sample_count * 100;

            // Generate regular low-overhead DPCs (15us - 90us)
            let base_duration = 20 + (sample_count % 60);
            let addr = if !drivers.is_empty() {
                drivers[(sample_count as usize) % drivers.len()].base_address + 0x100
            } else {
                default_driver_addr + 0x100
            };

            self.ingest_sample(ts, base_duration, addr, true);

            // Periodically generate ISRs
            if sample_count % 4 == 0 {
                self.ingest_sample(ts + 5, 12 + (sample_count % 30), addr, false);
            }

            // Inject latency spike if requested
            if inject_spike && sample_count == 5 {
                // Offending spike (1250us critical)
                self.ingest_sample(ts + 10, 1250, addr, true);
            }

            thread::sleep(Duration::from_millis(5));
        }

        self.is_running.store(false, Ordering::SeqCst);
    }
}

impl Drop for KernelLatencySessionManager {
    fn drop(&mut self) {
        self.stop_session();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_etw_elevation_check() {
        let elevated = KernelLatencySessionManager::is_elevation_available();
        println!("Process elevated for kernel ETW DPC tracing: {}", elevated);
    }

    #[test]
    fn test_synthetic_latency_session_lifecycle() {
        let mut session = KernelLatencySessionManager::new();
        assert!(!session.is_active());

        // Run synthetic test session for 200ms with intentional spike
        session.run_synthetic_session(Duration::from_millis(200), true);

        assert!(!session.is_active());
        assert_eq!(session.measurement_status(), LatencyMeasurementStatus::SyntheticTestFixture);
        assert_eq!(session.provenance().source, TelemetrySource::SyntheticFixture);
        assert!(session.total_dpcs() > 0, "Must capture DPC samples");
        assert!(session.total_isrs() > 0, "Must capture ISR samples");
        assert!(session.max_dpc_us() >= 1200, "Must detect the injected critical spike");

        // Verify monitor triggered warning events
        let warnings = session.monitor().get_warnings();
        assert!(!warnings.is_empty(), "Must emit warning event for > 1000us spike");
        assert_eq!(warnings[0].severity, val_opt_shared::models::latency::LatencySeverity::Critical);

        // Verify driver isolator recorded the spike
        let offending = session.isolator().get_offending_drivers();
        assert!(!offending.is_empty(), "Must isolate driver exceeding 500us");
        assert!(offending[0].max_execution_us >= 1200);
    }

    #[test]
    fn test_provenance_distinction() {
        let session = KernelLatencySessionManager::new();
        let prov = session.provenance();
        assert_eq!(prov.unit, "Microseconds (µs)");
        assert_eq!(prov.timestamp_source, "Win32 QueryPerformanceCounter (QPC)");
    }
}
