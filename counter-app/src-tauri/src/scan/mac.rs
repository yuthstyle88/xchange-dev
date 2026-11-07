//! CA-4b. The real scanner: ImageCaptureCore through `objc2-image-capture-core`. Proven on the
//! real Mac with the real scanner; no test here touches one.
//!
//! ImageCaptureCore delivers its delegate callbacks on the main thread's run loop (ruling on
//! #6391), so the main thread owns the `ICDeviceBrowser`, `ICScannerDevice`, session, functional
//! unit and delegate state (guarded by `objc2::MainThreadMarker`). `probe()` and `scan()` post to
//! the main run loop and await a `tokio::sync::oneshot`. Callbacks never block; file reads and
//! deletions run in `tokio::task::spawn_blocking`.

use std::cell::{OnceCell, RefCell, UnsafeCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::fs;
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_core_foundation::{
    CFRetained, CFRunLoop, CFRunLoopSource, CFRunLoopSourceContext, kCFRunLoopCommonModes,
    kCFRunLoopDefaultMode,
};
use objc2_foundation::{
    NSDictionary, NSError, NSNotFound, NSPoint, NSRect, NSString, NSUInteger, NSURL,
};
use objc2_image_capture_core::{
    ICDevice, ICDeviceBrowser, ICDeviceBrowserDelegate, ICDeviceDelegate, ICDeviceLocationTypeMask,
    ICDeviceStatus, ICDeviceTypeMask, ICLegacyReturnCode, ICLocalizedStatusNotificationKey,
    ICReturnCode, ICScannerBitDepth, ICScannerDevice, ICScannerDeviceDelegate,
    ICScannerFunctionalUnit, ICScannerFunctionalUnitType, ICScannerPixelDataType,
    ICScannerTransferMode, ICStatusNotificationKey,
};
use tokio::sync::oneshot;

use super::{ProbeFuture, ScanFailure, ScanFuture, ScanRequest, Scanner};

/// A cell whose contents may only be accessed with an [`objc2::MainThreadMarker`].
struct MainThreadCell<T>(UnsafeCell<T>);

// SAFETY: Every access requires a `MainThreadMarker`, which can only exist on the OS main thread.
unsafe impl<T> Send for MainThreadCell<T> {}
unsafe impl<T> Sync for MainThreadCell<T> {}

impl<T> MainThreadCell<T> {
    const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }

    fn get(&self, _mtm: MainThreadMarker) -> &T {
        // SAFETY: `_mtm` proves we are on the single OS main thread.
        unsafe { &*self.0.get() }
    }
}

struct SharedScanner {
    delegate: MainThreadCell<OnceCell<Retained<ScannerDelegate>>>,
}

impl SharedScanner {
    const fn new() -> Self {
        Self {
            delegate: MainThreadCell::new(OnceCell::new()),
        }
    }

    fn ensure_started(&self, mtm: MainThreadMarker) -> &Retained<ScannerDelegate> {
        self.delegate
            .get(mtm)
            .get_or_init(|| ScannerDelegate::new_and_start(mtm))
    }

    fn get(&self, mtm: MainThreadMarker) -> Option<&Retained<ScannerDelegate>> {
        self.delegate.get(mtm).get()
    }
}

pub struct MacScanner {
    shared: Arc<SharedScanner>,
    next_id: AtomicU64,
}

impl MacScanner {
    /// Schedules `ICDeviceBrowser` creation on the main run loop and returns at once.
    pub fn start() -> Self {
        let shared = Arc::new(SharedScanner::new());
        let init_shared = Arc::clone(&shared);
        post_to_main(move |mtm| {
            init_shared.ensure_started(mtm);
        });
        Self {
            shared,
            next_id: AtomicU64::new(1),
        }
    }
}

impl Scanner for MacScanner {
    fn probe(&self) -> ProbeFuture<'_> {
        let shared = Arc::clone(&self.shared);
        Box::pin(async move {
            let (reply_tx, reply_rx) = oneshot::channel();
            post_to_main(move |mtm| {
                let delegate = shared.ensure_started(mtm);
                handle_probe(delegate, reply_tx);
            });
            reply_rx
                .await
                .map_err(|_| ScanFailure::Device("scanner probe dropped reply".to_owned()))?
        })
    }

    fn scan(&self, request: ScanRequest) -> ScanFuture<'_> {
        let shared = Arc::clone(&self.shared);
        let scan_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        Box::pin(async move {
            let (reply_tx, reply_rx) = oneshot::channel();
            let start_shared = Arc::clone(&shared);
            post_to_main(move |mtm| {
                let delegate = start_shared.ensure_started(mtm);
                handle_start_scan(delegate, scan_id, request, reply_tx);
            });

            let path_result = match tokio::time::timeout(request.timeout, reply_rx).await {
                Ok(Ok(res)) => res,
                Ok(Err(_)) => Err(ScanFailure::Device("scanner dropped scan reply".to_owned())),
                Err(_) => {
                    let cancel_shared = Arc::clone(&shared);
                    post_to_main(move |mtm| {
                        if let Some(delegate) = cancel_shared.get(mtm) {
                            cancel_scan_by_id(delegate, scan_id);
                        }
                    });
                    Err(ScanFailure::Timeout)
                }
            };

            let path = path_result?;
            tokio::task::spawn_blocking(move || {
                let read_res = fs::read(&path);
                let _ = fs::remove_file(&path);
                match read_res {
                    Ok(bytes) if !bytes.is_empty() => Ok(bytes),
                    Ok(_) => Err(ScanFailure::Device(
                        "scanner produced empty JPEG file".to_owned(),
                    )),
                    Err(err) => Err(ScanFailure::Device(format!(
                        "failed reading scanned file: {err}"
                    ))),
                }
            })
            .await
            .map_err(|err| ScanFailure::Device(format!("scan file reader task failed: {err}")))?
        })
    }
}

type MainTask = Box<dyn FnOnce(MainThreadMarker) + Send>;

struct MainDispatcher {
    queue: Mutex<VecDeque<MainTask>>,
    run_loop: CFRetained<CFRunLoop>,
    source: CFRetained<CFRunLoopSource>,
}

// SAFETY: `CFRunLoop` and `CFRunLoopSource` are thread-safe CoreFoundation types for signaling
// and waking the main run loop from any thread.
unsafe impl Send for MainDispatcher {}
unsafe impl Sync for MainDispatcher {}

static MAIN_DISPATCHER: OnceLock<MainDispatcher> = OnceLock::new();

fn dispatcher() -> &'static MainDispatcher {
    MAIN_DISPATCHER.get_or_init(|| {
        let run_loop = CFRunLoop::main().expect("macOS main CFRunLoop exists");
        let mut ctx = CFRunLoopSourceContext {
            version: 0,
            info: ptr::null_mut(),
            retain: None,
            release: None,
            copyDescription: None,
            equal: None,
            hash: None,
            schedule: None,
            cancel: None,
            perform: Some(drain_main_tasks),
        };
        let source = unsafe { CFRunLoopSource::new(None, 0, &mut ctx) }
            .expect("creates main CFRunLoopSource");
        run_loop.add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
        run_loop.add_source(Some(&source), unsafe { kCFRunLoopDefaultMode });
        MainDispatcher {
            queue: Mutex::new(VecDeque::new()),
            run_loop,
            source,
        }
    })
}

fn post_to_main(task: impl FnOnce(MainThreadMarker) + Send + 'static) {
    let disp = dispatcher();
    disp.queue
        .lock()
        .expect("main task queue lock")
        .push_back(Box::new(task));
    disp.source.signal();
    disp.run_loop.wake_up();
}

unsafe extern "C-unwind" fn drain_main_tasks(_info: *mut c_void) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let disp = dispatcher();
    loop {
        let task = disp.queue.lock().expect("main task queue lock").pop_front();
        match task {
            Some(f) => f(mtm),
            None => break,
        }
    }
}

enum ScanPhase {
    WaitingForEnumeration,
    OpeningSession(Retained<ICScannerDevice>),
    WaitingForReady(Retained<ICScannerDevice>),
    SelectingFlatbed(Retained<ICScannerDevice>),
    Scanning(Retained<ICScannerDevice>),
}

impl ScanPhase {
    fn scanner(&self) -> Option<&ICScannerDevice> {
        match self {
            Self::WaitingForEnumeration => None,
            Self::OpeningSession(s)
            | Self::WaitingForReady(s)
            | Self::SelectingFlatbed(s)
            | Self::Scanning(s) => Some(s),
        }
    }
}

struct ActiveScan {
    id: u64,
    request: ScanRequest,
    phase: ScanPhase,
    scanned_path: Option<PathBuf>,
    reply: oneshot::Sender<Result<PathBuf, ScanFailure>>,
}

struct ScannerState {
    browser: Option<Retained<ICDeviceBrowser>>,
    enumerated_local: bool,
    scanners: Vec<Retained<ICScannerDevice>>,
    pending_probes: Vec<oneshot::Sender<Result<(), ScanFailure>>>,
    active_scan: Option<ActiveScan>,
}

impl ScannerState {
    fn new() -> Self {
        Self {
            browser: None,
            enumerated_local: false,
            scanners: Vec::new(),
            pending_probes: Vec::new(),
            active_scan: None,
        }
    }

    fn finish_active(&mut self, result: Result<PathBuf, ScanFailure>, cancel: bool) {
        let Some(mut active) = self.active_scan.take() else {
            return;
        };
        if let Some(scanner) = active.phase.scanner() {
            unsafe {
                if cancel {
                    scanner.cancelScan();
                }
                if scanner.hasOpenSession() {
                    scanner.requestCloseSession();
                }
            }
        }
        if result.is_err()
            && let Some(stale) = active.scanned_path.take()
        {
            cleanup_file_async(stale);
        }
        send_reply_or_cleanup(active.reply, result);
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RefCell<ScannerState>]
    struct ScannerDelegate;

    unsafe impl NSObjectProtocol for ScannerDelegate {}

    unsafe impl ICDeviceBrowserDelegate for ScannerDelegate {
        #[unsafe(method(deviceBrowser:didAddDevice:moreComing:))]
        fn device_browser_did_add_device(
            &self,
            _browser: &ICDeviceBrowser,
            device: &ICDevice,
            _more_coming: bool,
        ) {
            if let Ok(scanner) = device.retain().downcast::<ICScannerDevice>() {
                unsafe {
                    scanner.setDelegate(Some(ProtocolObject::from_ref(self)));
                }
                let mut state = self.ivars().borrow_mut();
                if !state.scanners.contains(&scanner) {
                    state.scanners.push(scanner.clone());
                }
                for probe in state.pending_probes.drain(..) {
                    let _ = probe.send(Ok(()));
                }
                if matches!(
                    state.active_scan.as_ref().map(|a| &a.phase),
                    Some(ScanPhase::WaitingForEnumeration)
                ) {
                    begin_session_on(&mut state, self, scanner);
                }
            }
        }

        #[unsafe(method(deviceBrowser:didRemoveDevice:moreGoing:))]
        fn device_browser_did_remove_device(
            &self,
            _browser: &ICDeviceBrowser,
            device: &ICDevice,
            _more_going: bool,
        ) {
            self.handle_device_removed(device);
        }

        #[unsafe(method(deviceBrowserDidEnumerateLocalDevices:))]
        fn device_browser_did_enumerate_local_devices(&self, browser: &ICDeviceBrowser) {
            let mut state = self.ivars().borrow_mut();
            sync_browser_devices(&mut state, browser, self);
            state.enumerated_local = true;
            let probe_res = if state.scanners.is_empty() {
                Err(ScanFailure::NoScanner)
            } else {
                Ok(())
            };
            for probe in state.pending_probes.drain(..) {
                let _ = probe.send(probe_res.clone());
            }
            if matches!(
                state.active_scan.as_ref().map(|a| &a.phase),
                Some(ScanPhase::WaitingForEnumeration)
            ) {
                if let Some(scanner) = state.scanners.first().cloned() {
                    begin_session_on(&mut state, self, scanner);
                } else {
                    state.finish_active(Err(ScanFailure::NoScanner), false);
                }
            }
        }
    }

    unsafe impl ICDeviceDelegate for ScannerDelegate {
        #[unsafe(method(didRemoveDevice:))]
        fn did_remove_device(&self, device: &ICDevice) {
            self.handle_device_removed(device);
        }

        #[unsafe(method(device:didOpenSessionWithError:))]
        fn device_did_open_session(&self, device: &ICDevice, error: Option<&NSError>) {
            let mut state = self.ivars().borrow_mut();
            let Some(scanner) = state.active_scan.as_ref().and_then(|active| match &active.phase {
                ScanPhase::OpeningSession(s) if (s.as_ref() as &ICDevice) == device => {
                    Some(s.clone())
                }
                _ => None,
            }) else {
                return;
            };
            if let Some(err) = error {
                state.finish_active(Err(map_nserror(err)), false);
                return;
            }
            if let Some(active) = state.active_scan.as_mut() {
                active.phase = ScanPhase::WaitingForReady(scanner);
            }
        }

        #[unsafe(method(deviceDidBecomeReady:))]
        fn device_did_become_ready(&self, device: &ICDevice) {
            let mut state = self.ivars().borrow_mut();
            let Some(scanner) = state.active_scan.as_ref().and_then(|active| match &active.phase {
                ScanPhase::WaitingForReady(s) if (s.as_ref() as &ICDevice) == device => {
                    Some(s.clone())
                }
                _ => None,
            }) else {
                return;
            };
            select_flatbed_or_fail(&mut state, &scanner);
        }

        #[unsafe(method(device:didCloseSessionWithError:))]
        fn device_did_close_session(&self, _device: &ICDevice, _error: Option<&NSError>) {}

        #[unsafe(method(device:didReceiveStatusInformation:))]
        fn device_did_receive_status(
            &self,
            device: &ICDevice,
            status: &NSDictionary<ICDeviceStatus, AnyObject>,
        ) {
            let mut state = self.ivars().borrow_mut();
            let matches_active = state
                .active_scan
                .as_ref()
                .and_then(|a| a.phase.scanner())
                .is_some_and(|s| (s as &ICDevice) == device);
            if matches_active && status_indicates_cover_open(status) {
                state.finish_active(Err(ScanFailure::CoverOpen), true);
            }
        }

        #[unsafe(method(device:didEncounterError:))]
        fn device_did_encounter_error(&self, device: &ICDevice, error: Option<&NSError>) {
            let Some(err) = error else {
                return;
            };
            let mut state = self.ivars().borrow_mut();
            let matches_active = state
                .active_scan
                .as_ref()
                .and_then(|a| a.phase.scanner())
                .is_some_and(|s| (s as &ICDevice) == device);
            if matches_active {
                state.finish_active(Err(map_nserror(err)), true);
            }
        }
    }

    unsafe impl ICScannerDeviceDelegate for ScannerDelegate {
        #[unsafe(method(scannerDevice:didSelectFunctionalUnit:error:))]
        fn scanner_did_select_functional_unit(
            &self,
            scanner: &ICScannerDevice,
            functional_unit: &ICScannerFunctionalUnit,
            error: Option<&NSError>,
        ) {
            let mut state = self.ivars().borrow_mut();
            let (scan_id, request, was_waiting_ready) = match state.active_scan.as_ref() {
                Some(active) => match &active.phase {
                    ScanPhase::WaitingForReady(s) if **s == *scanner => {
                        (active.id, active.request, true)
                    }
                    ScanPhase::SelectingFlatbed(s) if **s == *scanner => {
                        (active.id, active.request, false)
                    }
                    _ => return,
                },
                None => return,
            };

            if let Some(err) = error {
                state.finish_active(Err(map_nserror(err)), false);
                return;
            }

            let unit_type = unsafe { functional_unit.r#type() };
            if unit_type != ICScannerFunctionalUnitType::Flatbed {
                if was_waiting_ready {
                    select_flatbed_or_fail(&mut state, scanner);
                } else {
                    state.finish_active(Err(ScanFailure::Device("no flatbed".to_owned())), false);
                }
                return;
            }

            configure_and_request_scan(scanner, functional_unit, request, scan_id);
            if let Some(active) = state.active_scan.as_mut() {
                active.phase = ScanPhase::Scanning(scanner.retain());
            }
        }

        #[unsafe(method(scannerDevice:didScanToURL:))]
        fn scanner_did_scan_to_url(&self, scanner: &ICScannerDevice, url: &NSURL) {
            let path = url.path().map(|p| PathBuf::from(p.to_string()));
            let mut state = self.ivars().borrow_mut();
            let is_current_scanning = state.active_scan.as_ref().is_some_and(|active| {
                matches!(&active.phase, ScanPhase::Scanning(s) if **s == *scanner)
            });
            if !is_current_scanning {
                if let Some(stale) = path {
                    cleanup_file_async(stale);
                }
                return;
            }
            let Some(path) = path else {
                state.finish_active(
                    Err(ScanFailure::Device("scan URL had no file path".to_owned())),
                    false,
                );
                return;
            };
            let expected_prefix = doc_name_for(state.active_scan.as_ref().map_or(0, |a| a.id));
            let matches_id = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.starts_with(&expected_prefix));
            if !matches_id {
                cleanup_file_async(path);
                return;
            }
            if let Some(active) = state.active_scan.as_mut()
                && let Some(prev) = active.scanned_path.replace(path)
            {
                cleanup_file_async(prev);
            }
        }

        #[unsafe(method(scannerDevice:didCompleteScanWithError:))]
        fn scanner_did_complete_scan(&self, scanner: &ICScannerDevice, error: Option<&NSError>) {
            let mut state = self.ivars().borrow_mut();
            let is_current_scanning = state.active_scan.as_ref().is_some_and(|active| {
                matches!(&active.phase, ScanPhase::Scanning(s) if **s == *scanner)
            });
            if !is_current_scanning {
                return;
            }
            if let Some(err) = error {
                state.finish_active(Err(map_nserror(err)), false);
                return;
            }
            let path = state
                .active_scan
                .as_mut()
                .and_then(|active| active.scanned_path.take());
            match path {
                Some(p) => state.finish_active(Ok(p), false),
                None => state.finish_active(
                    Err(ScanFailure::Device(
                        "scanner produced no JPEG data".to_owned(),
                    )),
                    false,
                ),
            }
        }
    }
);

impl ScannerDelegate {
    fn new_and_start(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(ScannerState::new()));
        let delegate: Retained<Self> = unsafe { msg_send![super(this), init] };
        let browser = unsafe { ICDeviceBrowser::new() };
        unsafe {
            browser.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            browser.setBrowsedDeviceTypeMask(ICDeviceTypeMask(
                ICDeviceTypeMask::Scanner.0 | ICDeviceLocationTypeMask::Local.0,
            ));
            browser.start();
        }
        delegate.ivars().borrow_mut().browser = Some(browser);
        delegate
    }

    fn handle_device_removed(&self, device: &ICDevice) {
        let mut state = self.ivars().borrow_mut();
        state
            .scanners
            .retain(|s| (s.as_ref() as &ICDevice) != device);
        let removed_active = state
            .active_scan
            .as_ref()
            .and_then(|a| a.phase.scanner())
            .is_some_and(|s| (s as &ICDevice) == device);
        if removed_active {
            state.finish_active(Err(ScanFailure::NoScanner), false);
        }
    }
}

fn handle_probe(delegate: &ScannerDelegate, reply: oneshot::Sender<Result<(), ScanFailure>>) {
    let mut state = delegate.ivars().borrow_mut();
    if let Some(browser) = state.browser.clone() {
        sync_browser_devices(&mut state, &browser, delegate);
    }
    if !state.scanners.is_empty() {
        let _ = reply.send(Ok(()));
    } else if state.enumerated_local {
        let _ = reply.send(Err(ScanFailure::NoScanner));
    } else {
        state.pending_probes.push(reply);
    }
}

fn handle_start_scan(
    delegate: &ScannerDelegate,
    scan_id: u64,
    request: ScanRequest,
    reply: oneshot::Sender<Result<PathBuf, ScanFailure>>,
) {
    let mut state = delegate.ivars().borrow_mut();
    if state.active_scan.is_some() {
        let _ = reply.send(Err(ScanFailure::Busy));
        return;
    }
    if let Some(browser) = state.browser.clone() {
        sync_browser_devices(&mut state, &browser, delegate);
    }
    if state.scanners.is_empty() && state.enumerated_local {
        let _ = reply.send(Err(ScanFailure::NoScanner));
        return;
    }
    state.active_scan = Some(ActiveScan {
        id: scan_id,
        request,
        phase: ScanPhase::WaitingForEnumeration,
        scanned_path: None,
        reply,
    });
    if let Some(scanner) = state.scanners.first().cloned() {
        begin_session_on(&mut state, delegate, scanner);
    }
}

fn cancel_scan_by_id(delegate: &ScannerDelegate, scan_id: u64) {
    let mut state = delegate.ivars().borrow_mut();
    if state.active_scan.as_ref().is_some_and(|a| a.id == scan_id) {
        state.finish_active(Err(ScanFailure::Timeout), true);
    }
}

fn sync_browser_devices(
    state: &mut ScannerState,
    browser: &ICDeviceBrowser,
    delegate: &ScannerDelegate,
) {
    let Some(devices) = (unsafe { browser.devices() }) else {
        return;
    };
    for device in devices {
        if let Ok(scanner) = device.downcast::<ICScannerDevice>() {
            unsafe {
                scanner.setDelegate(Some(ProtocolObject::from_ref(delegate)));
            }
            if !state.scanners.contains(&scanner) {
                state.scanners.push(scanner);
            }
        }
    }
}

fn begin_session_on(
    state: &mut ScannerState,
    delegate: &ScannerDelegate,
    scanner: Retained<ICScannerDevice>,
) {
    unsafe {
        scanner.setDelegate(Some(ProtocolObject::from_ref(delegate)));
    }
    if unsafe { scanner.hasOpenSession() } {
        select_flatbed_or_fail(state, &scanner);
    } else if let Some(active) = state.active_scan.as_mut() {
        active.phase = ScanPhase::OpeningSession(scanner.clone());
        unsafe {
            scanner.requestOpenSession();
        }
    }
}

fn has_flatbed_unit(scanner: &ICScannerDevice) -> bool {
    let types = unsafe { scanner.availableFunctionalUnitTypes() };
    types
        .iter()
        .any(|num| num.unsignedIntegerValue() == ICScannerFunctionalUnitType::Flatbed.0)
}

fn select_flatbed_or_fail(state: &mut ScannerState, scanner: &ICScannerDevice) {
    if !has_flatbed_unit(scanner) {
        state.finish_active(Err(ScanFailure::Device("no flatbed".to_owned())), false);
        return;
    }
    if let Some(active) = state.active_scan.as_mut() {
        active.phase = ScanPhase::SelectingFlatbed(scanner.retain());
        unsafe {
            scanner.requestSelectFunctionalUnit(ICScannerFunctionalUnitType::Flatbed);
        }
    }
}

fn doc_name_for(scan_id: u64) -> String {
    format!("xc-scan-{}-{scan_id}", std::process::id())
}

fn configure_and_request_scan(
    scanner: &ICScannerDevice,
    unit: &ICScannerFunctionalUnit,
    request: ScanRequest,
    scan_id: u64,
) {
    let temp_dir = std::env::temp_dir();
    let dir_str = NSString::from_str(&temp_dir.to_string_lossy());
    let dir_url = NSURL::fileURLWithPath_isDirectory(&dir_str, true);
    let doc_name = NSString::from_str(&doc_name_for(scan_id));

    unsafe {
        let pixel_type = if request.color {
            ICScannerPixelDataType::RGB
        } else {
            ICScannerPixelDataType::Gray
        };
        unit.setPixelDataType(pixel_type);
        unit.setBitDepth(ICScannerBitDepth::Depth8Bits);

        let target_dpi = request.dpi as NSUInteger;
        let supported = unit.supportedResolutions();
        let dpi = if supported.containsIndex(target_dpi) {
            target_dpi
        } else {
            let ge = supported.indexGreaterThanOrEqualToIndex(target_dpi);
            if ge != NSNotFound as NSUInteger {
                ge
            } else {
                let le = supported.indexLessThanOrEqualToIndex(target_dpi);
                if le != NSNotFound as NSUInteger {
                    le
                } else {
                    target_dpi
                }
            }
        };
        unit.setResolution(dpi);

        if request.whole_bed {
            let size = unit.physicalSize();
            unit.setScanArea(NSRect::new(NSPoint::new(0.0, 0.0), size));
        }

        scanner.setTransferMode(ICScannerTransferMode::FileBased);
        scanner.setDownloadsDirectory(&dir_url);
        scanner.setDocumentName(&doc_name);
        scanner.setDocumentUTI(&NSString::from_str("public.jpeg"));
        scanner.requestScan();
    }
}

fn cleanup_file_async(path: PathBuf) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn_blocking(move || {
            let _ = fs::remove_file(path);
        });
    } else {
        std::thread::spawn(move || {
            let _ = fs::remove_file(path);
        });
    }
}

fn send_reply_or_cleanup(
    reply: oneshot::Sender<Result<PathBuf, ScanFailure>>,
    result: Result<PathBuf, ScanFailure>,
) {
    if let Err(Ok(path)) = reply.send(result) {
        cleanup_file_async(path);
    }
}

fn status_indicates_cover_open(status: &NSDictionary<ICDeviceStatus, AnyObject>) -> bool {
    for key in unsafe { [ICStatusNotificationKey, ICLocalizedStatusNotificationKey] } {
        if let Some(obj) = status.objectForKey(key)
            && let Some(s) = obj.downcast_ref::<NSString>()
            && is_cover_open_text(&s.to_string())
        {
            return true;
        }
    }
    false
}

/// Whole-word match for `("cover" | "lid") && "open"`, so substrings like `"invalid"` or
/// `"discover"` never match.
pub fn is_cover_open_text(text: &str) -> bool {
    let mut has_cover_or_lid = false;
    let mut has_open = false;
    for word in text.split(|c: char| !c.is_ascii_alphabetic()) {
        if word.eq_ignore_ascii_case("cover") || word.eq_ignore_ascii_case("lid") {
            has_cover_or_lid = true;
        } else if word.eq_ignore_ascii_case("open") {
            has_open = true;
        }
    }
    has_cover_or_lid && has_open
}

fn map_nserror(err: &NSError) -> ScanFailure {
    let code = err.code();
    if code == ICReturnCode::CommunicationTimedOut.0 {
        return ScanFailure::Timeout;
    }
    if code == ICReturnCode::ScannerInUseByLocalUser.0
        || code == ICReturnCode::ScannerInUseByRemoteUser.0
    {
        return ScanFailure::Busy;
    }
    if code == ICLegacyReturnCode::DeviceNotFoundErr.0 {
        return ScanFailure::NoScanner;
    }
    let msg = err.localizedDescription().to_string();
    if is_cover_open_text(&msg) {
        return ScanFailure::CoverOpen;
    }
    ScanFailure::Device(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn send_reply_or_cleanup_deletes_file_when_receiver_dropped() {
        let temp_path = std::env::temp_dir().join(format!(
            "test_scan_drop_{}_{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&temp_path, b"passport image").expect("write temp file");
        assert!(temp_path.exists());

        let (tx, rx) = oneshot::channel();
        drop(rx);
        send_reply_or_cleanup(tx, Ok(temp_path.clone()));

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while temp_path.exists() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            !temp_path.exists(),
            "temp file must be deleted when receiver is dropped"
        );
    }

    #[tokio::test]
    async fn send_reply_or_cleanup_preserves_file_when_receiver_alive() {
        let temp_path = std::env::temp_dir().join(format!(
            "test_scan_keep_{}_{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&temp_path, b"passport image").expect("write temp file");
        assert!(temp_path.exists());

        let (tx, rx) = oneshot::channel();
        send_reply_or_cleanup(tx, Ok(temp_path.clone()));

        let res = rx.await.expect("receive reply");
        assert_eq!(res, Ok(temp_path.clone()));
        assert!(
            temp_path.exists(),
            "temp file must not be deleted when received"
        );
        let _ = fs::remove_file(temp_path);
    }
}
