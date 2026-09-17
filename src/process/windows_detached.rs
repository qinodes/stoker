//! Windows-specific detached helper spawning.

use std::io;
use std::os::windows::io::AsRawHandle;
use std::process::{Child, Command};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{
    GetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation,
};

// Standard-handle inheritance is process-global on Windows. Serialize the
// short interval in which the launcher handles are made non-inheritable.
static SPAWN_LOCK: Mutex<()> = Mutex::new(());

pub(super) fn spawn(command: &mut Command) -> io::Result<Child> {
    let _lock = SPAWN_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _stdio = StandardHandleInheritance::disable()?;
    command.spawn()
}

struct StandardHandleInheritance {
    changed: Vec<HANDLE>,
}

impl StandardHandleInheritance {
    fn disable() -> io::Result<Self> {
        let handles = [
            std::io::stdin().as_raw_handle(),
            std::io::stdout().as_raw_handle(),
            std::io::stderr().as_raw_handle(),
        ];
        let mut changed = Vec::new();

        for raw in handles {
            let handle = raw as HANDLE;
            if !valid(handle) || changed.contains(&handle) {
                continue;
            }
            let mut flags = 0;
            // Some hosts expose console pseudo-handles that do not support
            // handle-information queries. They cannot be inherited as the
            // redirected pipe handles involved in this bug, so skip them.
            if unsafe { GetHandleInformation(handle, &mut flags) } == 0
                || flags & HANDLE_FLAG_INHERIT == 0
            {
                continue;
            }
            if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
                restore(&changed);
                return Err(io::Error::last_os_error());
            }
            changed.push(handle);
        }

        Ok(Self { changed })
    }
}

impl Drop for StandardHandleInheritance {
    fn drop(&mut self) {
        restore(&self.changed);
    }
}

fn valid(handle: HANDLE) -> bool {
    !handle.is_null() && handle != INVALID_HANDLE_VALUE
}

fn restore(handles: &[HANDLE]) {
    for &handle in handles {
        // Best effort during cleanup: a spawn error should remain the
        // caller-visible error even if the host closed a standard handle.
        unsafe {
            SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
        }
    }
}
