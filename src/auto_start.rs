//! Auto start using the Windows Task Scheduler.

use std::{
    any::TypeId, env::current_exe, ffi::OsStr, os::windows::process::CommandExt, process::Command,
    rc::Rc, sync::Arc, time::Duration,
};

use crate::{
    dynamic_gui::DynamicUiHooks,
    settings::{AutoStart, UiSettings},
    tray::{SystemTray, TrayPlugin},
};

pub fn change_install(should_install: bool) -> Result<(), String> {
    // Note: Task Scheduler paths must use backslashes (but runas can't
    // escape them correctly for schtasks, so don't use them)
    let task_name = "Lej77's VirtualDesktopManager - Elevated Auto Start".to_string();
    let was_installed = is_installed(&task_name)
        .map_err(|e| format!("Failed to check if elevated auto start was installed: {e}"))?;

    if was_installed == should_install {
        return Ok(());
    }

    if should_install {
        let exe_path =
            current_exe().map_err(|e| format!("failed to resolve the executable's path: {e}"))?;
        install(&task_name, exe_path.as_ref())
            .map_err(|e| format!("Failed to install elevated auto start: {e}"))?;
    } else {
        uninstall(&task_name)
            .map_err(|e| format!("Failed to uninstall elevated auto start: {e}"))?;
    }

    // Wait for changes to be applied:
    std::thread::sleep(Duration::from_millis(2000));

    let was_installed = is_installed(&task_name)
        .map_err(|e| format!("Failed to check if elevated auto start was installed: {e}"))?;
    if was_installed == should_install {
        Ok(())
    } else {
        Err(format!(
            "failed to {} the task \"{task_name}\" {} the Task Scheduler",
            if should_install { "create" } else { "remove" },
            if should_install { "in" } else { "from" }
        ))
    }
}

pub fn is_installed(task_name: &str) -> Result<bool, String> {
    let output = Command::new("schtasks")
        .args(["/Query", "/TN"])
        .arg(task_name)
        // Hide console window:
        // https://stackoverflow.com/questions/6371149/what-is-the-difference-between-detach-process-and-create-no-window-process-creat
        // https://learn.microsoft.com/sv-se/windows/win32/procthread/process-creation-flags?redirectedfrom=MSDN
        .creation_flags(/*DETACHED_PROCESS*/ 0x00000008)
        .output()
        .map_err(|e| format!("failed to run schtasks: {e}"))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        code => Err(format!(
            "failed to check if the task \"{task_name}\" existed in the Task Scheduler{}\n\nStderr:{}",
            if let Some(code) = code {
                format!(" (exit code: {code})")
            } else {
                "".to_string()
            },
            String::from_utf8_lossy(&output.stderr)
        )),
    }
}

pub fn install(task_name: &str, program_path: &OsStr) -> Result<(), String> {
    // 1. Creating a task that uses the `Highest` `RunLevel` will fail if we
    //    don't have admin rights so we run this command with sudo.
    // 2. We use "powershell" instead of "schtasks" to create the task since
    //    some task settings aren't exposed as cli flags for "schtasks".
    //   - The settings in question are:
    //     - The task is terminated after 3 days
    //     - The task is only started if the PC is connected to a power
    //       supply.
    //   - Another workaround would be to use "schtasks" XML import option.
    //     - This would require writing a temp file that included the path
    //       to the program that should be started.
    //
    // Info about powershell code:
    // https://learn.microsoft.com/en-us/powershell/module/scheduledtasks/register-scheduledtask?view=windowsserver2022-ps
    // https://stackoverflow.com/questions/2157554/how-to-handle-command-line-arguments-in-powershell
    let _status = runas::Command::new("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-WindowStyle")
        .arg("Hidden")
        .arg("-Command")
        // Inline the powershell script that we want to run (alternatively
        // we could store the code as a file and pass a path to it, but
        // passing the code directly makes it easier to inspect in the UAC
        // prompt):
        .arg(format!(
            "& {{{}}}",
            include_str!("./install-elevated-autostart.ps1")
        ))
        // Task name:
        .arg(format!("\"{task_name}\""))
        // Path to started program:
        .arg(
            // If path has spaces then it must be surrounded by quotes,
            // otherwise anything after the first space will be interpreted
            // as arguments to the started program:
            format!(
                "\"{}\"",
                program_path
                    .to_str()
                    .ok_or("program path wasn't valid UTF-8")?
                    // schtasks doesn't handle the escaped backslashes
                    // correctly so avoid them:
                    .replace('\\', "/")
            ),
        )
        // Task description:
        .arg("\"Start Virtual Desktop Manager at startup\"")
        // Show the admin prompt:
        .gui(true)
        // But hide the created schtasks window:
        .show(false)
        .status()
        .map_err(|e| format!("failed to start \"powershell\": {e}"))?;
    Ok(())
    // Status code is always -1?
    // See: https://github.com/mitsuhiko/rust-runas/issues/13
    // Related to refactor away from C glue code in:
    // https://github.com/mitsuhiko/rust-runas/commit/220624592f8202107592b83c943aad73bd3142b0
    /*
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "failed to create the task \"{task_name}\" in the Task Scheduler{}",
            if let Some(code) = status.code() {
                format!(" (exit code: {code})")
            } else {
                "".to_string()
            }
        ))
    }
    */
}

pub fn uninstall(task_name: &str) -> Result<(), String> {
    if task_name.contains('*') {
        return Err(
            "don't use * inside task names, they will be interpreted as wildcards".to_string(),
        );
    }
    let _status = runas::Command::new("schtasks")
        .arg("/Delete")
        // Task name:
        .arg("/TN")
        .arg(task_name)
        // Force: skips "are you sure" prompt:
        .arg("/F")
        // Show the admin prompt:
        .gui(true)
        // But hide the created schtasks window:
        .show(false)
        .status()
        .map_err(|e| format!("failed to run schtasks: {e}"))?;
    Ok(())
    // Status code is always -1?
    /*
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "failed to delete the task \"{task_name}\" in the Task Scheduler{}",
            if let Some(code) = status.code() {
                format!(" (exit code: {code})")
            } else {
                "".to_string()
            }
        ))
    }
    */
}

/// This plugin tracks UI settings.
#[derive(nwd::NwgPartial, Default)]
pub struct AutoStartPlugin {}
impl AutoStartPlugin {
    fn update_installed(&self, tray_ui: &SystemTray) {
        if cfg!(debug_assertions) {
            return;
        }
        // TODO(perf): do this in a background thread.
        // TODO(feat): support non elevated auto start.
        let res = change_install(tray_ui.settings().get().auto_start != AutoStart::Disabled);
        if let Err(e) = res {
            tray_ui.show_notification("Virtual Desktop Manager Error", &e);
        }
    }
}
impl DynamicUiHooks<SystemTray> for AutoStartPlugin {
    fn before_partial_build(
        &mut self,
        _tray_ui: &Rc<SystemTray>,
        _should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)> {
        None
    }
    fn after_partial_build(&mut self, tray_ui: &Rc<SystemTray>) {
        self.update_installed(tray_ui);
    }
}
impl TrayPlugin for AutoStartPlugin {
    fn on_settings_changed(
        &self,
        tray_ui: &Rc<SystemTray>,
        prev: &Arc<UiSettings>,
        new: &Arc<UiSettings>,
    ) {
        if prev.auto_start != new.auto_start {
            self.update_installed(tray_ui);
        }
    }
}
