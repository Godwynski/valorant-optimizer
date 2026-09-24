//! Real-Time Latency Health Monitor.
//!
//! Maintains a high-performance in-memory ring buffer of recent DPC/ISR durations.
//! Evaluates incoming interrupt durations against competitive gaming thresholds:
//! - Warning: 500µs - 999µs (Noticeable input delay risk)
//! - Critical: >= 1000µs (Direct cause of frame drops, micro-stutter, and packet jitter)
//!
//! Emits structured `LatencyWarningEvent` alerts without locking or degrading game threads.

use std::sync::RwLock;
use tracing::{warn, debug};
use val_opt_shared::models::latency::{DpcIsrSample, LatencySeverity, LatencyWarningEvent};

pub const DEFAULT_MONITOR_RING_CAPACITY: usize = 4096;
pub const WARNING_THRESHOLD_US: u64 = 500;
pub const CRITICAL_THRESHOLD_US: u64 = 1000;

/// Ring buffer storing recent DPC/ISR samples with O(1) ingestion.
pub struct LatencyRingBuffer {
    samples: Vec<Option<DpcIsrSample>>,
    capacity: usize,
    head: usize,
    count: usize,
}

impl LatencyRingBuffer {
    pub fn new(capacity: usize) -> Self {
        let mut samples = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            samples.push(None);
        }
        Self {
            samples,
            capacity,
            head: 0,
            count: 0,
        }
    }

    pub fn push(&mut self, sample: DpcIsrSample) {
        self.samples[self.head] = Some(sample);
        self.head = (self.head + 1) % self.capacity;
        if self.count < self.capacity {
            self.count += 1;
        }
    }

    pub fn to_vec(&self) -> Vec<DpcIsrSample> {
        let mut result = Vec::with_capacity(self.count);
        if self.count == 0 {
            return result;
        }

        let start = if self.count < self.capacity {
            0
        } else {
            self.head
        };

        for i in 0..self.count {
            let idx = (start + i) % self.capacity;
            if let Some(ref sample) = self.samples[idx] {
                result.push(sample.clone());
            }
        }
        result
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn clear(&mut self) {
        for item in &mut self.samples {
            *item = None;
        }
        self.head = 0;
        self.count = 0;
    }
}

/// The continuous background interrupt health monitor.
pub struct LatencyMonitor {
    ring_buffer: RwLock<LatencyRingBuffer>,
    warning_events: RwLock<Vec<LatencyWarningEvent>>,
    max_warnings_history: usize,
}

impl LatencyMonitor {
    pub fn new(capacity: usize) -> Self {
        Self {
            ring_buffer: RwLock::new(LatencyRingBuffer::new(capacity)),
            warning_events: RwLock::new(Vec::new()),
            max_warnings_history: 1000,
        }
    }

    /// Ingest a DPC/ISR sample into the ring buffer.
    /// If the duration breaches thresholds (>= 500µs warning or >= 1000µs critical),
    /// generates and records a `LatencyWarningEvent`.
    pub fn ingest_sample(&self, sample: DpcIsrSample) -> Option<LatencyWarningEvent> {
        let duration = sample.duration_us;
        let mut warning = None;

        if duration >= WARNING_THRESHOLD_US {
            let severity = LatencySeverity::from_duration_us(duration);
            let driver_name = sample.driver_name.clone().unwrap_or_else(|| "Unknown".to_string());
            let threshold = if severity == LatencySeverity::Critical {
                CRITICAL_THRESHOLD_US
            } else {
                WARNING_THRESHOLD_US
            };

            let event = LatencyWarningEvent {
                timestamp_us: sample.timestamp_us,
                duration_us: duration,
                driver_name: driver_name.clone(),
                routine_address: sample.routine_address,
                is_dpc: sample.is_dpc,
                threshold_us: threshold,
                severity,
                message: format!(
                    "{} routine in '{}' executed for {}µs (exceeds {}µs threshold)",
                    if sample.is_dpc { "DPC" } else { "ISR" },
                    driver_name,
                    duration,
                    threshold
                ),
            };

            if severity == LatencySeverity::Critical {
                warn!(
                    driver = %driver_name,
                    duration_us = duration,
                    routine = format_args!("0x{:X}", sample.routine_address),
                    "CRITICAL INTERRUPT LATENCY SPIKE DETECTED"
                );
            } else {
                debug!(
                    driver = %driver_name,
                    duration_us = duration,
                    "Interrupt latency warning"
                );
            }

            let mut warnings = self.warning_events.write().unwrap();
            if warnings.len() >= self.max_warnings_history {
                warnings.remove(0);
            }
            warnings.push(event.clone());
            warning = Some(event);
        }

        // Store sample in ring buffer
        let mut ring = self.ring_buffer.write().unwrap();
        ring.push(sample);

        warning
    }

    /// Retrieve all recorded warning events.
    pub fn get_warnings(&self) -> Vec<LatencyWarningEvent> {
        let warnings = self.warning_events.read().unwrap();
        warnings.clone()
    }

    /// Retrieve the most recent N samples from the ring buffer.
    pub fn get_recent_samples(&self, max_count: usize) -> Vec<DpcIsrSample> {
        let ring = self.ring_buffer.read().unwrap();
        let all = ring.to_vec();
        if all.len() > max_count {
            all[all.len() - max_count..].to_vec()
        } else {
            all
        }
    }

    /// Clear all stored samples and warnings.
    pub fn clear(&self) {
        self.ring_buffer.write().unwrap().clear();
        self.warning_events.write().unwrap().clear();
    }
}

impl Default for LatencyMonitor {
    fn default() -> Self {
        Self::new(DEFAULT_MONITOR_RING_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_fifo_rollover() {
        let mut rb = LatencyRingBuffer::new(3);
        assert_eq!(rb.count(), 0);

        rb.push(DpcIsrSample { timestamp_us: 1, duration_us: 10, routine_address: 0, is_dpc: true, driver_name: None });
        rb.push(DpcIsrSample { timestamp_us: 2, duration_us: 20, routine_address: 0, is_dpc: true, driver_name: None });
        rb.push(DpcIsrSample { timestamp_us: 3, duration_us: 30, routine_address: 0, is_dpc: true, driver_name: None });
        assert_eq!(rb.count(), 3);

        let vec = rb.to_vec();
        assert_eq!(vec.len(), 3);
        assert_eq!(vec[0].duration_us, 10);
        assert_eq!(vec[2].duration_us, 30);

        // Fourth push rolls over oldest item (10)
        rb.push(DpcIsrSample { timestamp_us: 4, duration_us: 40, routine_address: 0, is_dpc: true, driver_name: None });
        assert_eq!(rb.count(), 3);
        let rolled = rb.to_vec();
        assert_eq!(rolled[0].duration_us, 20);
        assert_eq!(rolled[1].duration_us, 30);
        assert_eq!(rolled[2].duration_us, 40);
    }

    #[test]
    fn test_monitor_threshold_alerts() {
        let monitor = LatencyMonitor::new(100);

        // Normal sample (80us)
        let w1 = monitor.ingest_sample(DpcIsrSample {
            timestamp_us: 1000,
            duration_us: 80,
            routine_address: 0x1000,
            is_dpc: true,
            driver_name: Some("ndis.sys".to_string()),
        });
        assert!(w1.is_none());
        assert_eq!(monitor.get_warnings().len(), 0);

        // Warning sample (650us)
        let w2 = monitor.ingest_sample(DpcIsrSample {
            timestamp_us: 2000,
            duration_us: 650,
            routine_address: 0x2000,
            is_dpc: true,
            driver_name: Some("nvlddmkm.sys".to_string()),
        });
        assert!(w2.is_some());
        let event = w2.unwrap();
        assert_eq!(event.severity, LatencySeverity::Warning);
        assert_eq!(event.threshold_us, 500);

        // Critical sample (1250us >= 1000us)
        let w3 = monitor.ingest_sample(DpcIsrSample {
            timestamp_us: 3000,
            duration_us: 1250,
            routine_address: 0x3000,
            is_dpc: true,
            driver_name: Some("audio.sys".to_string()),
        });
        assert!(w3.is_some());
        let crit_event = w3.unwrap();
        assert_eq!(crit_event.severity, LatencySeverity::Critical);
        assert_eq!(crit_event.threshold_us, 1000);
        assert_eq!(monitor.get_warnings().len(), 2);
    }
}
