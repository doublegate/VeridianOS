//! Print Spooler Service
//!
//! Manages print queues, jobs, and printer configurations. Supports virtual
//! printers and IPP/1.1 protocol stubs for future network printing.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};

// ---------------------------------------------------------------------------
// Print job
// ---------------------------------------------------------------------------

/// Unique job identifier.
pub type PrintJobId = u64;

/// Status of a print job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintJobStatus {
    /// Waiting in the queue.
    Queued,
    /// Currently being printed.
    Printing,
    /// Successfully completed.
    Completed,
    /// Failed with an error.
    Failed,
    /// Cancelled by the user.
    Cancelled,
}

/// Paper size for printing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaperSize {
    /// US Letter (8.5 x 11 in).
    #[default]
    Letter,
    /// ISO A4 (210 x 297 mm).
    A4,
    /// US Legal (8.5 x 14 in).
    Legal,
}

impl PaperSize {
    /// Width in points (1/72 inch).
    pub fn width_pts(&self) -> u32 {
        match self {
            Self::Letter => 612,
            Self::A4 => 595,
            Self::Legal => 612,
        }
    }

    /// Height in points (1/72 inch).
    pub fn height_pts(&self) -> u32 {
        match self {
            Self::Letter => 792,
            Self::A4 => 842,
            Self::Legal => 1008,
        }
    }
}

/// A print job.
#[derive(Debug, Clone)]
pub struct PrintJob {
    /// Unique identifier.
    pub id: PrintJobId,
    /// Name of the document being printed.
    pub document_name: String,
    /// Raw document data.
    pub data: Vec<u8>,
    /// Current status.
    pub status: PrintJobStatus,
    /// Number of copies requested.
    pub copies: u32,
    /// Total number of pages (0 = unknown).
    pub pages: u32,
    /// Page range start (1-based, 0 = all).
    pub page_start: u32,
    /// Page range end (0 = all).
    pub page_end: u32,
}

impl PrintJob {
    /// Create a new queued print job.
    pub fn new(id: PrintJobId, document_name: &str, data: Vec<u8>) -> Self {
        Self {
            id,
            document_name: String::from(document_name),
            data,
            status: PrintJobStatus::Queued,
            copies: 1,
            pages: 0,
            page_start: 0,
            page_end: 0,
        }
    }

    /// Data size in bytes.
    pub fn data_size(&self) -> usize {
        self.data.len()
    }
}

// ---------------------------------------------------------------------------
// Print queue
// ---------------------------------------------------------------------------

/// A queue of print jobs for a single printer.
#[derive(Debug)]
pub struct PrintQueue {
    /// Queued jobs (front = next to print).
    jobs: Vec<PrintJob>,
    /// Maximum number of jobs allowed in the queue.
    max_jobs: usize,
    /// Total number of jobs ever submitted.
    total_submitted: u64,
    /// Total number of jobs completed.
    total_completed: u64,
}

impl PrintQueue {
    /// Create a new queue with the given capacity.
    pub fn new(max_jobs: usize) -> Self {
        Self {
            jobs: Vec::new(),
            max_jobs,
            total_submitted: 0,
            total_completed: 0,
        }
    }

    /// Enqueue a job. Returns false if the queue is full.
    pub fn enqueue(&mut self, job: PrintJob) -> bool {
        if self.jobs.len() >= self.max_jobs {
            return false;
        }
        self.jobs.push(job);
        self.total_submitted += 1;
        true
    }

    /// Dequeue the next job (FIFO).
    pub fn dequeue(&mut self) -> Option<PrintJob> {
        if self.jobs.is_empty() {
            None
        } else {
            let mut job = self.jobs.remove(0);
            job.status = PrintJobStatus::Printing;
            Some(job)
        }
    }

    /// Cancel a job by ID. Returns true if found and cancelled.
    pub fn cancel(&mut self, job_id: PrintJobId) -> bool {
        for job in &mut self.jobs {
            if job.id == job_id && job.status == PrintJobStatus::Queued {
                job.status = PrintJobStatus::Cancelled;
                return true;
            }
        }
        false
    }

    /// Get the status of a specific job.
    pub fn get_status(&self, job_id: PrintJobId) -> Option<PrintJobStatus> {
        self.jobs.iter().find(|j| j.id == job_id).map(|j| j.status)
    }

    /// Number of jobs currently in the queue.
    pub fn pending_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|j| j.status == PrintJobStatus::Queued)
            .count()
    }

    /// Total number of jobs in the queue (all statuses).
    pub fn total_count(&self) -> usize {
        self.jobs.len()
    }

    /// Mark a job as completed and update stats.
    pub fn complete_job(&mut self, job_id: PrintJobId) -> bool {
        for job in &mut self.jobs {
            if job.id == job_id {
                job.status = PrintJobStatus::Completed;
                self.total_completed += 1;
                return true;
            }
        }
        false
    }

    /// Remove completed and cancelled jobs from the queue.
    pub fn purge_finished(&mut self) {
        self.jobs.retain(|j| {
            j.status != PrintJobStatus::Completed && j.status != PrintJobStatus::Cancelled
        });
    }
}

// ---------------------------------------------------------------------------
// Printer configuration
// ---------------------------------------------------------------------------

/// Printer driver type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrinterDriver {
    /// Virtual printer (writes to file or memory).
    #[default]
    Virtual,
    /// IPP network printer.
    Ipp,
    /// Raw passthrough (direct data to device).
    Raw,
}

/// Configuration for a single printer.
#[derive(Debug, Clone)]
pub struct PrinterConfig {
    /// Printer name (unique identifier).
    pub name: String,
    /// Driver type.
    pub driver_type: PrinterDriver,
    /// Output resolution in DPI.
    pub resolution_dpi: u32,
    /// Default paper size.
    pub paper_size: PaperSize,
    /// Whether this printer is enabled.
    pub enabled: bool,
    /// Whether this is the default printer.
    pub is_default: bool,
}

impl PrinterConfig {
    /// Create a new printer configuration.
    pub fn new(name: &str, driver_type: PrinterDriver) -> Self {
        Self {
            name: String::from(name),
            driver_type,
            resolution_dpi: 300,
            paper_size: PaperSize::Letter,
            enabled: true,
            is_default: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Print spooler
// ---------------------------------------------------------------------------

/// Central print spooler managing multiple printer queues.
#[derive(Debug)]
pub struct PrintSpooler {
    /// Queues indexed by printer name.
    queues: BTreeMap<String, PrintQueue>,
    /// Printer configurations indexed by name.
    printers: BTreeMap<String, PrinterConfig>,
    /// Next job ID to assign.
    next_job_id: PrintJobId,
    /// Default printer name.
    default_printer: Option<String>,
}

impl Default for PrintSpooler {
    fn default() -> Self {
        Self::new()
    }
}

impl PrintSpooler {
    /// Create a new empty spooler.
    pub fn new() -> Self {
        Self {
            queues: BTreeMap::new(),
            printers: BTreeMap::new(),
            next_job_id: 1,
            default_printer: None,
        }
    }

    /// Add a printer to the spooler.
    pub fn add_printer(&mut self, config: PrinterConfig) {
        let name = config.name.clone();
        let is_default = config.is_default;
        self.printers.insert(name.clone(), config);
        self.queues.insert(name.clone(), PrintQueue::new(100));
        if is_default || self.default_printer.is_none() {
            self.default_printer = Some(name);
        }
    }

    /// Remove a printer by name.
    pub fn remove_printer(&mut self, name: &str) -> bool {
        let removed = self.printers.remove(name).is_some();
        self.queues.remove(name);
        if self.default_printer.as_deref() == Some(name) {
            self.default_printer = self.printers.keys().next().cloned();
        }
        removed
    }

    /// Submit a job to a specific printer (or the default).
    pub fn submit_job(
        &mut self,
        printer: Option<&str>,
        document_name: &str,
        data: Vec<u8>,
    ) -> Option<PrintJobId> {
        let printer_name = printer
            .map(String::from)
            .or_else(|| self.default_printer.clone())?;

        let queue = self.queues.get_mut(&printer_name)?;

        let id = self.next_job_id;
        self.next_job_id += 1;
        let job = PrintJob::new(id, document_name, data);

        if queue.enqueue(job) {
            Some(id)
        } else {
            None
        }
    }

    /// List all jobs for a printer.
    pub fn list_jobs(&self, printer: &str) -> Vec<&PrintJob> {
        self.queues
            .get(printer)
            .map(|q| q.jobs.iter().collect())
            .unwrap_or_default()
    }

    /// Cancel a job.
    pub fn cancel_job(&mut self, printer: &str, job_id: PrintJobId) -> bool {
        self.queues
            .get_mut(printer)
            .map(|q| q.cancel(job_id))
            .unwrap_or(false)
    }

    /// Get the number of printers.
    pub fn printer_count(&self) -> usize {
        self.printers.len()
    }

    /// Get the default printer name.
    pub fn default_printer(&self) -> Option<&str> {
        self.default_printer.as_deref()
    }

    /// Get a printer configuration by name.
    pub fn get_printer(&self, name: &str) -> Option<&PrinterConfig> {
        self.printers.get(name)
    }
}

// ---------------------------------------------------------------------------
// IPP/1.1 protocol stubs
// ---------------------------------------------------------------------------

/// IPP operation codes (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum IppOperation {
    /// Print a new job.
    PrintJob = 0x0002,
    /// Validate a job before submission.
    ValidateJob = 0x0004,
    /// Create a job (without data).
    CreateJob = 0x0005,
    /// Get job attributes.
    GetJobAttributes = 0x0009,
    /// Get printer attributes.
    GetPrinterAttributes = 0x000B,
    /// Cancel a job.
    CancelJob = 0x0008,
}

/// IPP status codes (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum IppStatus {
    /// Successful.
    Ok = 0x0000,
    /// Client error: bad request.
    ClientBadRequest = 0x0400,
    /// Client error: not found.
    ClientNotFound = 0x0406,
    /// Server error: internal.
    ServerInternal = 0x0500,
}

/// An IPP request.
#[derive(Debug, Clone)]
pub struct IppRequest {
    /// IPP version (major, minor).
    pub version: (u8, u8),
    /// Operation code.
    pub operation: IppOperation,
    /// Request ID.
    pub request_id: u32,
    /// Operation attributes (key-value pairs).
    pub attributes: BTreeMap<String, String>,
}

impl IppRequest {
    /// Create a new request.
    pub fn new(operation: IppOperation, request_id: u32) -> Self {
        Self {
            version: (1, 1),
            operation,
            request_id,
            attributes: BTreeMap::new(),
        }
    }
}

/// An IPP response.
#[derive(Debug, Clone)]
pub struct IppResponse {
    /// IPP version (major, minor).
    pub version: (u8, u8),
    /// Status code.
    pub status: IppStatus,
    /// Request ID (matches the request).
    pub request_id: u32,
    /// Response attributes.
    pub attributes: BTreeMap<String, String>,
}

impl IppResponse {
    /// Create a success response.
    pub fn ok(request_id: u32) -> Self {
        Self {
            version: (1, 1),
            status: IppStatus::Ok,
            request_id,
            attributes: BTreeMap::new(),
        }
    }

    /// Create an error response.
    pub fn error(request_id: u32, status: IppStatus) -> Self {
        Self {
            version: (1, 1),
            status,
            request_id,
            attributes: BTreeMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_print_job_new() {
        let job = PrintJob::new(1, "test.pdf", vec![0u8; 100]);
        assert_eq!(job.id, 1);
        assert_eq!(job.status, PrintJobStatus::Queued);
        assert_eq!(job.data_size(), 100);
    }

    #[test]
    fn test_print_queue_enqueue_dequeue() {
        let mut queue = PrintQueue::new(10);
        let job = PrintJob::new(1, "test.pdf", vec![]);
        assert!(queue.enqueue(job));
        assert_eq!(queue.pending_count(), 1);
        let dequeued = queue.dequeue().unwrap();
        assert_eq!(dequeued.id, 1);
        assert_eq!(dequeued.status, PrintJobStatus::Printing);
    }

    #[test]
    fn test_print_queue_full() {
        let mut queue = PrintQueue::new(1);
        assert!(queue.enqueue(PrintJob::new(1, "a", vec![])));
        assert!(!queue.enqueue(PrintJob::new(2, "b", vec![])));
    }

    #[test]
    fn test_print_queue_cancel() {
        let mut queue = PrintQueue::new(10);
        queue.enqueue(PrintJob::new(1, "test", vec![]));
        assert!(queue.cancel(1));
        assert_eq!(queue.get_status(1), Some(PrintJobStatus::Cancelled));
    }

    #[test]
    fn test_spooler_add_printer() {
        let mut spooler = PrintSpooler::new();
        let config = PrinterConfig::new("pdf-printer", PrinterDriver::Virtual);
        spooler.add_printer(config);
        assert_eq!(spooler.printer_count(), 1);
        assert_eq!(spooler.default_printer(), Some("pdf-printer"));
    }

    #[test]
    fn test_spooler_submit_job() {
        let mut spooler = PrintSpooler::new();
        spooler.add_printer(PrinterConfig::new("lp0", PrinterDriver::Virtual));
        let id = spooler.submit_job(Some("lp0"), "doc.pdf", vec![1, 2, 3]);
        assert!(id.is_some());
        let jobs = spooler.list_jobs("lp0");
        assert_eq!(jobs.len(), 1);
    }

    #[test]
    fn test_spooler_default_printer() {
        let mut spooler = PrintSpooler::new();
        let id = spooler.submit_job(None, "doc", vec![]);
        assert!(id.is_none()); // No default printer
        spooler.add_printer(PrinterConfig::new("lp0", PrinterDriver::Virtual));
        let id = spooler.submit_job(None, "doc", vec![]);
        assert!(id.is_some());
    }

    #[test]
    fn test_paper_size() {
        assert_eq!(PaperSize::Letter.width_pts(), 612);
        assert_eq!(PaperSize::A4.height_pts(), 842);
    }
}
