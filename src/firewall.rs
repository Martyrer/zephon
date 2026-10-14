//! Windows Firewall check and one-time UAC install for this exe.
//!
//! Receivers send NTP timing requests (UDP) to the sender from a random port, and use TCP for
//! the event channel. Without inbound allow rules for this exe's path covering both, Windows
//! drops them and the receiver answers the stream SETUP with 400 after about 30 s. The rules
//! are path-specific: `target\debug`, `target\release` and an installed copy each need their own.
//!
//! Both the check and the install are our own variants of Nyx `netutil::firewall` (same
//! NetSecurity PowerShell, JSON output). The check also reads each rule's protocol, because
//! Windows' first-run "Allow access" prompt makes one TCP and one UDP rule, and a lone TCP rule
//! passes Nyx's check but still drops the timing replies. The install is ours because Nyx
//! quotes the exe path into the elevated script text (a crafted folder name could run code as
//! administrator), starts `powershell.exe` by search, and removes every copy's rule by name.
//!
//! Paths never appear in script text here: `ps_literal` carries them as base64, and
//! PowerShell is started from the System32 path, not looked up.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Default, PartialEq)]
pub enum FirewallState {
    #[default]
    Unknown,
    Checking,
    Ok,
    /// No inbound allow rule for both TCP and UDP (`tcp`/`udp`: what the allow rules cover),
    /// or a block rule wins over them.
    Missing {
        blocked: bool,
        tcp: bool,
        udp: bool,
    },
    Installing,
    /// The UAC prompt was declined.
    Cancelled,
    Error(String),
}

impl FirewallState {
    pub fn needs_rule(&self) -> bool {
        matches!(self, Self::Missing { .. } | Self::Cancelled)
    }
}

/// What the firewall does with inbound traffic to this exe.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// At least one firewall profile (Domain/Private/Public) is enabled.
    pub firewall_enabled: bool,
    /// Enabled inbound allow rules cover TCP / UDP (a rule for any protocol covers both).
    pub tcp: bool,
    pub udp: bool,
    /// An enabled inbound block rule exists for the exe (overrides allow rules).
    pub blocked: bool,
}

impl Report {
    /// True when inbound traffic to the exe is likely to be dropped.
    pub fn needs_fix(&self) -> bool {
        self.firewall_enabled && (self.blocked || !(self.tcp && self.udp))
    }
}

// NetSecurity enum values: Direction Inbound=1; Action Allow=2, Block=4.
const DIRECTION_INBOUND: i64 = 1;
const ACTION_ALLOW: i64 = 2;
const ACTION_BLOCK: i64 = 4;

#[derive(Deserialize)]
struct RawProfile {
    #[serde(rename = "Enabled")]
    enabled: i64,
}

#[derive(Deserialize)]
struct RawRule {
    #[serde(rename = "Direction")]
    direction: i64,
    #[serde(rename = "Action")]
    action: i64,
    #[serde(rename = "Enabled")]
    enabled: i64,
    /// "Any", "TCP", "UDP", "ICMPv4", or a protocol number.
    #[serde(rename = "Protocol", default)]
    protocol: Option<String>,
}

#[derive(Deserialize)]
struct RawReport {
    #[serde(default)]
    profiles: OneOrMany<RawProfile>,
    #[serde(default)]
    rules: OneOrMany<RawRule>,
}

/// `ConvertTo-Json` emits a bare object for single-element arrays and `null` for empty ones.
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    Many(Vec<T>),
    One(T),
    None(()),
}

impl<T> Default for OneOrMany<T> {
    fn default() -> Self {
        OneOrMany::None(())
    }
}

impl<T> OneOrMany<T> {
    fn into_vec(self) -> Vec<T> {
        match self {
            OneOrMany::Many(v) => v,
            OneOrMany::One(t) => vec![t],
            OneOrMany::None(_) => Vec::new(),
        }
    }
}

/// Parses the JSON printed by [`check_script`].
fn parse_report(json: &str) -> Result<Report, String> {
    let json = json.trim();
    if json.is_empty() {
        return Err("empty PowerShell output".into());
    }
    let raw: RawReport = serde_json::from_str(json).map_err(|e| format!("bad JSON: {e}"))?;
    // Windows always has three profiles. None at all means the query failed, and reading that
    // as "firewall off" would report OK while traffic is still dropped.
    let profiles = raw.profiles.into_vec();
    if profiles.is_empty() {
        return Err("no firewall profile data".into());
    }
    let mut report = Report {
        firewall_enabled: profiles.iter().any(|p| p.enabled == 1),
        ..Default::default()
    };
    for r in raw.rules.into_vec() {
        if r.direction != DIRECTION_INBOUND || r.enabled != 1 {
            continue;
        }
        match r.action {
            ACTION_BLOCK => report.blocked = true,
            ACTION_ALLOW => match r
                .protocol
                .as_deref()
                .map(str::to_ascii_uppercase)
                .as_deref()
            {
                Some("TCP" | "6") => report.tcp = true,
                Some("UDP" | "17") => report.udp = true,
                Some("ANY") | None => (report.tcp, report.udp) = (true, true),
                Some(_) => {}
            },
            _ => {}
        }
    }
    Ok(report)
}

/// A PowerShell expression that evaluates to `s`, carried as base64 of its UTF-16LE bytes so
/// nothing in `s` is ever parsed as script. Quote doubling is not enough for file paths:
/// PowerShell also ends a single-quoted string at the curly quotes U+2018..U+201B. The base64
/// alphabet has no quotes, `$`, backticks or semicolons.
fn ps_literal(s: &str) -> String {
    let b64 = netutil::firewall::encode_command(s);
    format!("[Text.Encoding]::Unicode.GetString([Convert]::FromBase64String('{b64}'))")
}

/// PowerShell function `Get-ExeRules $path`: the rules whose program is exactly `$path`.
/// `-Program` takes a wildcard pattern (a `[` in a folder name changes what it matches), so
/// the path is escaped and the matches compared exactly. "No filter for this program" is the
/// normal empty case; any other error is thrown so the caller fails instead of seeing no rules.
const GET_EXE_RULES: &str = "function Get-ExeRules($path){\
     $f=@(Get-NetFirewallApplicationFilter -Program ([Management.Automation.WildcardPattern]::Escape($path)) -ErrorAction SilentlyContinue -ErrorVariable qe);\
     foreach($e in $qe){if($e.CategoryInfo.Category -ne 'ObjectNotFound'){throw $e}};\
     @($f | Where-Object {$_.Program -eq $path} | Get-NetFirewallRule)};";

/// PowerShell that prints `{"profiles":[...],"rules":[...]}` for `program`, with each rule's
/// protocol from its port filter. Any query error ends it with exit code 1 and the message on
/// stderr, so a failed query is never read as "no rules" or "firewall off".
fn check_script(program: &str) -> String {
    let prog = ps_literal(program);
    format!(
        "$ErrorActionPreference='Stop';try{{\
         $prog={prog};{GET_EXE_RULES}\
         $p=@(Get-NetFirewallProfile | ForEach-Object {{ @{{Enabled=[int]($_.Enabled -eq 'True')}} }});\
         $r=@(Get-ExeRules $prog | ForEach-Object {{ $pf=$_ | Get-NetFirewallPortFilter; @{{Direction=[int]$_.Direction;Action=[int]$_.Action;Enabled=[int]($_.Enabled -eq 'True');Protocol=[string]$pf.Protocol}} }});\
         @{{profiles=$p;rules=$r}} | ConvertTo-Json -Depth 3 -Compress\
         }}catch{{[Console]::Error.WriteLine($_.Exception.Message);exit 1}}"
    )
}

/// Display name of the allow rule for `program`. The full path keeps copies apart (Nyx used
/// the file name only, so every copy shared one name).
fn rule_name(program: &str) -> String {
    format!("Zephon - {program}")
}

/// Elevated PowerShell for `program`: removes its inbound block rules and the allow rules an
/// earlier install made (`name`, or Nyx's `legacy_name`), then adds one inbound allow rule for
/// all protocols and profiles. Only rules whose program is exactly this path are touched.
/// Exits 0 on success, 1 on failure.
fn install_script(program: &str, name: &str, legacy_name: &str) -> String {
    let (prog, name, legacy) = (
        ps_literal(program),
        ps_literal(name),
        ps_literal(legacy_name),
    );
    format!(
        "$ErrorActionPreference='Stop';try{{\
         $prog={prog};$name={name};$legacy={legacy};{GET_EXE_RULES}\
         Get-ExeRules $prog | Where-Object {{ $_.Direction -eq 'Inbound' -and ($_.Action -eq 'Block' -or $_.DisplayName -eq $name -or $_.DisplayName -eq $legacy) }} | Remove-NetFirewallRule;\
         New-NetFirewallRule -DisplayName $name -Direction Inbound -Program $prog -Action Allow -Profile Any -Enabled True | Out-Null;\
         exit 0}}catch{{exit 1}}"
    )
}

/// PowerShell flags for every run: no profile scripts, no prompts.
const PS_FLAGS: &str = "-NoProfile -NonInteractive";

/// How long to wait for the elevated script after the UAC prompt was accepted.
const INSTALL_TIMEOUT_MS: u32 = 60_000;

pub fn exe_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("zephon.exe"))
}

/// Queries the firewall for this exe (no admin rights, about 1 s; runs PowerShell).
#[cfg(windows)]
pub fn check() -> Result<Report, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = check_script(&exe_path().to_string_lossy());
    let (powershell, system_dir) = sys::powershell_exe()?;
    let out = std::process::Command::new(powershell)
        .args(PS_FLAGS.split(' '))
        .arg("-EncodedCommand")
        .arg(netutil::firewall::encode_command(&script))
        .current_dir(system_dir)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("powershell: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "PowerShell failed ({}): {}",
            out.status,
            stderr.trim()
        ));
    }
    parse_report(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(windows))]
pub fn check() -> Result<Report, String> {
    Err("not on Windows".into())
}

pub fn state_from(result: Result<Report, String>) -> FirewallState {
    match result {
        Ok(r) if r.needs_fix() => FirewallState::Missing {
            blocked: r.blocked,
            tcp: r.tcp,
            udp: r.udp,
        },
        Ok(_) => FirewallState::Ok,
        Err(e) => FirewallState::Error(format!("firewall check failed: {e}")),
    }
}

/// Shows one UAC prompt and creates an inbound allow rule (all protocols, so TCP and UDP;
/// profile Any) for this exe only, removing block rules for it. Then re-checks.
/// The rule is named `Zephon - <full exe path>`; other copies' rules are left alone.
pub fn install() -> FirewallState {
    match install_elevated(&exe_path()) {
        Ok(true) => state_from(check()),
        Ok(false) => FirewallState::Cancelled,
        Err(e) => FirewallState::Error(format!("firewall rule install failed: {e}")),
    }
}

/// The program path the rule is made for. A relative path (`exe_path`'s fallback) would make a
/// rule for nothing useful, and a lossy path would make one for a different file.
fn rule_program(exe: &Path) -> Result<&str, String> {
    if !exe.is_absolute() {
        return Err(format!("exe path is not absolute: {}", exe.display()));
    }
    exe.to_str()
        .ok_or_else(|| "exe path is not valid Unicode".into())
}

/// Runs [`install_script`] for `exe` as administrator. `Ok(false)`: the UAC prompt was declined.
fn install_elevated(exe: &Path) -> Result<bool, String> {
    let program = rule_program(exe)?;
    let script = install_script(
        program,
        &rule_name(program),
        &netutil::firewall::rule_name(program),
    );
    let params = format!(
        "{PS_FLAGS} -WindowStyle Hidden -EncodedCommand {}",
        netutil::firewall::encode_command(&script)
    );
    let (powershell, system_dir) = sys::powershell_exe()?;
    match sys::run_elevated(&powershell, &params, &system_dir, INSTALL_TIMEOUT_MS)? {
        None => Ok(false),
        Some(0) => Ok(true),
        Some(code) => Err(format!("firewall script exited with code {code}")),
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};

    type Handle = isize;
    const SEE_MASK_NOCLOSEPROCESS: u32 = 0x40;
    /// Errors come back as `ShellExecuteEx` failures instead of message boxes, which the CLI
    /// (`zephon firewall --install`) and the controller's background thread have no use for.
    const SEE_MASK_FLAG_NO_UI: u32 = 0x400;
    const SW_HIDE: i32 = 0;
    const ERROR_CANCELLED: i32 = 1223;
    const WAIT_OBJECT_0: u32 = 0;

    /// `SHELLEXECUTEINFOW`. shellapi.h packs it to 1 byte on 32-bit only, where every field is
    /// 4 bytes anyway, so `repr(C)` matches on both; the assert below checks the documented sizes.
    #[repr(C)]
    struct ShellExecuteInfoW {
        size: u32,
        mask: u32,
        hwnd: Handle,
        verb: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
        inst_app: Handle,
        id_list: *mut std::ffi::c_void,
        class: *const u16,
        hkey_class: Handle,
        hot_key: u32,
        icon_or_monitor: Handle,
        process: Handle,
    }

    const _: () = assert!(
        size_of::<ShellExecuteInfoW>()
            == if cfg!(target_pointer_width = "64") {
                112
            } else {
                60
            }
    );

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
        fn WaitForSingleObject(handle: Handle, millis: u32) -> u32;
        fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
    }

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Full path of Windows PowerShell and the System32 folder. A bare `powershell.exe` is
    /// looked up in the app's own folder (`Command`) or the current folder (`ShellExecuteEx`)
    /// first, where a planted copy would run, possibly as administrator.
    pub fn powershell_exe() -> Result<(PathBuf, PathBuf), String> {
        let mut buf = vec![0u16; 260];
        loop {
            // SAFETY: `buf` has `buf.len()` writable u16s.
            let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
            if n == 0 {
                return Err(format!(
                    "GetSystemDirectory: {}",
                    std::io::Error::last_os_error()
                ));
            }
            // On success `n` excludes the NUL; when the buffer is too small it is the size needed.
            if n < buf.len() {
                buf.truncate(n);
                break;
            }
            buf.resize(n, 0);
        }
        let system_dir = PathBuf::from(OsString::from_wide(&buf));
        Ok((
            system_dir.join(r"WindowsPowerShell\v1.0\powershell.exe"),
            system_dir,
        ))
    }

    /// Starts `file` as administrator (one UAC prompt), hidden, and waits for it.
    /// `Ok(None)`: the prompt was declined; `Ok(Some(code))`: the process exit code.
    pub fn run_elevated(
        file: &Path,
        params: &str,
        dir: &Path,
        timeout_ms: u32,
    ) -> Result<Option<u32>, String> {
        let (verb, params) = (wide("runas".as_ref()), wide(params.as_ref()));
        let (file, dir) = (wide(file.as_os_str()), wide(dir.as_os_str()));
        let mut info = ShellExecuteInfoW {
            size: size_of::<ShellExecuteInfoW>() as u32,
            mask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
            hwnd: 0,
            verb: verb.as_ptr(),
            file: file.as_ptr(),
            parameters: params.as_ptr(),
            directory: dir.as_ptr(),
            show: SW_HIDE,
            inst_app: 0,
            id_list: std::ptr::null_mut(),
            class: std::ptr::null(),
            hkey_class: 0,
            hot_key: 0,
            icon_or_monitor: 0,
            process: 0,
        };
        // SAFETY: `info` is fully initialised and the NUL-terminated strings it points to
        // outlive the call.
        if unsafe { ShellExecuteExW(&mut info) } == 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(ERROR_CANCELLED) {
                return Ok(None);
            }
            return Err(format!("ShellExecuteEx: {e}"));
        }
        if info.process == 0 {
            return Err("ShellExecuteEx started no process".into());
        }
        let mut code = 0u32;
        // SAFETY: `info.process` is a process handle we own (SEE_MASK_NOCLOSEPROCESS) and is
        // closed exactly once, here.
        let (waited, got_code) = unsafe {
            let waited = WaitForSingleObject(info.process, timeout_ms);
            let got_code = GetExitCodeProcess(info.process, &mut code);
            CloseHandle(info.process);
            (waited, got_code)
        };
        if waited != WAIT_OBJECT_0 {
            let secs = timeout_ms / 1000;
            return Err(format!(
                "the elevated PowerShell did not finish within {secs} s"
            ));
        }
        if got_code == 0 {
            return Err("could not read the elevated PowerShell's exit code".into());
        }
        Ok(Some(code))
    }
}

#[cfg(not(windows))]
mod sys {
    use std::path::{Path, PathBuf};

    pub fn powershell_exe() -> Result<(PathBuf, PathBuf), String> {
        Err("not on Windows".into())
    }
    pub fn run_elevated(_: &Path, _: &str, _: &Path, _: u32) -> Result<Option<u32>, String> {
        Err("not on Windows".into())
    }
}

/// Blocking: check, for the CLI and the controller.
pub fn check_state() -> FirewallState {
    state_from(check())
}

pub fn describe(r: &Report) -> String {
    format!(
        "firewall enabled: {}, inbound allow rule: TCP {}, UDP {}, inbound block rule: {} -> {}",
        r.firewall_enabled,
        r.tcp,
        r.udp,
        r.blocked,
        if r.needs_fix() { "NEEDS FIX" } else { "OK" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_rules_must_cover_tcp_and_udp() {
        // Windows' "Allow access" prompt: one TCP and one UDP rule.
        let both = parse_report(
            r#"{"profiles":[{"Enabled":1}],"rules":[{"Direction":1,"Action":2,"Enabled":1,"Protocol":"UDP"},{"Direction":1,"Action":2,"Enabled":1,"Protocol":"TCP"}]}"#,
        )
        .unwrap();
        assert!(both.tcp && both.udp && !both.needs_fix());
        // Nyx's install: one rule for any protocol (a bare object, not an array).
        let any = parse_report(
            r#"{"profiles":{"Enabled":1},"rules":{"Direction":1,"Action":2,"Enabled":1,"Protocol":"Any"}}"#,
        )
        .unwrap();
        assert!(!any.needs_fix());
        // TCP only: the timing replies (UDP) are still dropped.
        let tcp = parse_report(
            r#"{"profiles":[{"Enabled":1}],"rules":[{"Direction":1,"Action":2,"Enabled":1,"Protocol":"TCP"},{"Direction":1,"Action":2,"Enabled":0,"Protocol":"UDP"}]}"#,
        )
        .unwrap();
        assert!(tcp.tcp && !tcp.udp && tcp.needs_fix());
        let missing_udp = FirewallState::Missing {
            blocked: false,
            tcp: true,
            udp: false,
        };
        assert_eq!(state_from(Ok(tcp)), missing_udp);
    }

    #[test]
    fn block_rules_win_and_a_disabled_firewall_needs_nothing() {
        let blocked = parse_report(
            r#"{"profiles":[{"Enabled":1}],"rules":[{"Direction":1,"Action":2,"Enabled":1,"Protocol":"Any"},{"Direction":1,"Action":4,"Enabled":1,"Protocol":"UDP"}]}"#,
        )
        .unwrap();
        assert!(blocked.blocked && blocked.needs_fix());
        let off = parse_report(r#"{"profiles":[{"Enabled":0}],"rules":null}"#).unwrap();
        assert!(!off.needs_fix());
    }

    #[test]
    fn empty_or_garbled_output_is_a_specific_error() {
        for out in ["", " \r\n"] {
            assert_eq!(
                parse_report(out),
                Err("empty PowerShell output".into()),
                "{out:?}"
            );
        }
        let garbled = parse_report("WARNING: something").unwrap_err();
        assert!(garbled.starts_with("bad JSON"), "{garbled}");
    }

    /// A report with all profiles on and `rules` (JSON objects without the braces).
    fn report_for(rules: &[&str]) -> Report {
        let rules: Vec<String> = rules.iter().map(|r| format!("{{{r}}}")).collect();
        parse_report(&format!(
            r#"{{"profiles":[{{"Enabled":1}},{{"Enabled":1}},{{"Enabled":1}}],"rules":[{}]}}"#,
            rules.join(",")
        ))
        .unwrap()
    }

    #[test]
    fn outbound_and_disabled_rules_do_not_count() {
        let none = Report {
            firewall_enabled: true,
            ..Default::default()
        };
        // Direction 2 is Outbound: an outbound allow rule lets nothing in, an outbound block
        // rule blocks nothing inbound.
        assert_eq!(
            report_for(&[r#""Direction":2,"Action":2,"Enabled":1,"Protocol":"Any""#]),
            none
        );
        assert_eq!(
            report_for(&[r#""Direction":2,"Action":4,"Enabled":1,"Protocol":"Any""#]),
            none
        );
        // Enabled 0: a disabled block rule blocks nothing.
        let allowed = report_for(&[
            r#""Direction":1,"Action":2,"Enabled":1,"Protocol":"Any""#,
            r#""Direction":1,"Action":4,"Enabled":0,"Protocol":"Any""#,
        ]);
        assert!(!allowed.blocked && !allowed.needs_fix(), "{allowed:?}");
    }

    #[test]
    fn protocols_match_by_name_or_number_in_any_case() {
        let covers = |rule: &str| {
            let r = report_for(&[rule]);
            assert!(!r.blocked, "{rule}");
            (r.tcp, r.udp)
        };
        let allow = |protocol: &str| {
            covers(&format!(
                r#""Direction":1,"Action":2,"Enabled":1,"Protocol":"{protocol}""#
            ))
        };
        for (protocol, tcp_udp) in [
            ("TCP", (true, false)),
            ("tcp", (true, false)),
            ("6", (true, false)),
            ("UDP", (false, true)),
            ("Udp", (false, true)),
            ("17", (false, true)),
            ("Any", (true, true)),
            ("any", (true, true)),
            ("ICMPv4", (false, false)),
            ("58", (false, false)),
            ("", (false, false)),
        ] {
            assert_eq!(allow(protocol), tcp_udp, "{protocol:?}");
        }
        // No port filter data: the rule is not limited to a protocol.
        assert_eq!(
            covers(r#""Direction":1,"Action":2,"Enabled":1"#),
            (true, true)
        );
        // Action 0 (NotConfigured) or anything unknown neither allows nor blocks.
        for action in [0, 1, 3] {
            let rule = format!(r#""Direction":1,"Action":{action},"Enabled":1,"Protocol":"Any""#);
            assert_eq!(covers(&rule), (false, false), "{rule}");
        }
    }

    #[test]
    fn one_enabled_profile_means_the_firewall_is_on() {
        let r = parse_report(
            r#"{"profiles":[{"Enabled":0},{"Enabled":1},{"Enabled":0}],"rules":null}"#,
        )
        .unwrap();
        assert!(r.firewall_enabled && r.needs_fix(), "{r:?}");
    }

    #[cfg(windows)]
    #[test]
    fn rule_program_must_be_absolute_and_unicode() {
        let ok = r"C:\Program Files\Zephon\zephon.exe";
        assert_eq!(rule_program(Path::new(ok)), Ok(ok));
        // `exe_path`'s fallback, and paths relative to the current drive or its folder.
        for relative in [
            "zephon.exe",
            r"target\zephon.exe",
            r"\zephon.exe",
            "C:zephon.exe",
        ] {
            let err = rule_program(Path::new(relative)).unwrap_err();
            assert!(err.contains("not absolute"), "{relative}: {err}");
        }
        // A lone surrogate: `to_string_lossy` would turn it into U+FFFD, a different file.
        use std::os::windows::ffi::OsStringExt;
        let mut wide: Vec<u16> = r"C:\z\".encode_utf16().collect();
        wide.extend([0xD800, u16::from(b'x')]);
        let lossy = PathBuf::from(std::ffi::OsString::from_wide(&wide));
        assert_eq!(
            rule_program(&lossy),
            Err("exe path is not valid Unicode".into())
        );
    }

    #[test]
    fn missing_profile_data_is_an_error_not_a_disabled_firewall() {
        // All three profiles off: a valid report, nothing to fix.
        let off = parse_report(
            r#"{"profiles":[{"Enabled":0},{"Enabled":0},{"Enabled":0}],"rules":null}"#,
        )
        .unwrap();
        assert!(!off.firewall_enabled && !off.needs_fix());
        // No profile data: the query failed, so the state is unknown.
        for json in [
            r#"{"profiles":null,"rules":null}"#,
            r#"{"profiles":[],"rules":[]}"#,
            "{}",
        ] {
            assert!(parse_report(json).is_err(), "{json}");
            assert!(
                matches!(state_from(parse_report(json)), FirewallState::Error(_)),
                "{json}"
            );
        }
    }

    /// Decodes standard base64 (the app only encodes, so this lives in the tests).
    fn base64_decode(s: &str) -> Vec<u8> {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let (mut bits, mut n, mut out) = (0u32, 0, Vec::new());
        for c in s.bytes().filter(|&c| c != b'=') {
            let v = ALPHABET
                .iter()
                .position(|&a| a == c)
                .expect("base64 character");
            bits = (bits << 6) | v as u32;
            n += 6;
            if n >= 8 {
                n -= 8;
                out.push((bits >> n) as u8);
                bits &= (1 << n) - 1;
            }
        }
        out
    }

    /// The strings carried by every [`ps_literal`] in `script`, decoded the way PowerShell does.
    fn decoded_literals(script: &str) -> Vec<String> {
        script
            .split("FromBase64String('")
            .skip(1)
            .map(|rest| {
                let bytes = base64_decode(rest.split_once('\'').unwrap().0);
                let (pairs, odd) = bytes.as_chunks::<2>();
                assert!(odd.is_empty(), "odd UTF-16LE byte count");
                let units: Vec<u16> = pairs.iter().map(|&p| u16::from_le_bytes(p)).collect();
                String::from_utf16(&units).unwrap()
            })
            .collect()
    }

    /// A folder name built to break out of a quoted PowerShell string: ASCII and curly single
    /// quotes (PowerShell treats U+2018..U+201B as `'`), a subexpression, backticks, `;`, `[`.
    const HOSTILE: &str =
        "C:\\Users\\O'Neil\\a\u{2019};\u{2018}b\u{201A}\u{201B}\\$(Get-Date)\\`x`;y[1]\\zephon.exe";

    #[test]
    fn paths_never_reach_script_text() {
        let legacy = netutil::firewall::rule_name(HOSTILE);
        let install = install_script(HOSTILE, &rule_name(HOSTILE), &legacy);
        for script in [check_script(HOSTILE), install.clone()] {
            assert!(!script.contains(HOSTILE));
            assert!(!script.contains("Neil") && !script.contains("$(Get-Date)"));
            // Curly quotes are not ASCII, so this also rules them out anywhere in the script.
            assert!(script.is_ascii());
        }
        assert_eq!(decoded_literals(&check_script(HOSTILE)), [HOSTILE]);
        assert_eq!(
            decoded_literals(&install),
            [HOSTILE.to_string(), format!("Zephon - {HOSTILE}"), legacy]
        );
    }

    #[test]
    fn literals_round_trip() {
        // Lengths 0..3 cover every base64 padding case; U+1F3B5 is a surrogate pair.
        let quotes = "\u{2018}\u{2019}\u{201A}\u{201B}'`;$()";
        for s in [HOSTILE, "", "a", "ab", "abc", quotes, "\u{1F3B5}"] {
            let lit = ps_literal(s);
            assert!(lit.is_ascii());
            assert_eq!(decoded_literals(&lit), [s]);
        }
    }

    /// Runs `script` in Windows PowerShell 5.1 (the System32 copy, as the app does) with the
    /// test runner's rights, which on CI are an administrator's. So the checks in [`STUBS`], not
    /// the token, are what keep a script under test off the real firewall.
    #[cfg(windows)]
    fn powershell(script: &str, env: &[(&str, &str)]) -> std::process::Output {
        let (powershell, system_dir) = sys::powershell_exe().unwrap();
        std::process::Command::new(powershell)
            .args(["-NoProfile", "-NonInteractive", "-EncodedCommand"])
            .arg(netutil::firewall::encode_command(script))
            .current_dir(system_dir)
            .envs(env.iter().copied())
            .output()
            .unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn powershell_decodes_literals_to_the_exact_string() {
        let quotes = "'\u{2018}\u{2019}\u{201A}\u{201B}\"\u{201C}\u{201D}";
        let strings = [
            HOSTILE,
            "",
            "a",
            "ab",
            "abc",
            quotes,
            "$x $(Remove-Item x) ${env:PATH} @(1)",
            "`n`$`'``",
            "a;b|c&d",
            "line 1\nline 2\r\n\ttab",
            "\u{1F3B5} C:\\M\u{00FC}sik\\\u{97F3}\u{697D}",
            "#<# '@\n'@",
        ];
        // Each decoded string as its UTF-16 code units, so nothing depends on console encoding.
        let script = strings
            .iter()
            .map(|s| {
                format!(
                    "$s={};[Console]::Out.WriteLine('['+((@($s.ToCharArray())|ForEach-Object{{[int]$_}}) -join ',')+']');",
                    ps_literal(s)
                )
            })
            .collect::<String>();
        let out = powershell(&script, &[]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).unwrap();
        let got: Vec<&str> = stdout.lines().collect();
        let want: Vec<String> = strings
            .iter()
            .map(|s| {
                let units: Vec<String> = s.encode_utf16().map(|u| u.to_string()).collect();
                format!("[{}]", units.join(","))
            })
            .collect();
        assert_eq!(got, want);
    }

    /// The harness itself: a script that could reach the firewall without a cmdlet, through a
    /// scriptblock, reflection, COM or CIM, is refused before any of it runs. Each payload
    /// only prints "ran", so a harness that let one through fails here without harm.
    #[cfg(windows)]
    #[test]
    fn the_harness_refuses_scripts_that_go_around_the_stubs() {
        let ran = "[Console]::Out.WriteLine('ran')";
        let scripts = [
            format!("[scriptblock]::Create(\"{ran}\").Invoke()"),
            format!("$ExecutionContext.InvokeCommand.InvokeScript(\"{ran}\")"),
            "[Console]::Out.GetType().GetMethod('WriteLine', [type[]]@([string])).Invoke([Console]::Out, @('ran'))"
                .to_string(),
            format!(
                "[Activator]::CreateInstance([type]::GetTypeFromProgID('Scripting.Dictionary')) | Out-Null; {ran}"
            ),
            format!(
                "#Requires -Version 5
{ran}"
            ),
            format!("New-Object Collections.ArrayList | Out-Null; {ran}"),
        ];
        for script in &scripts {
            let out = powershell(
                &format!(
                    "{STUBS}
{script}"
                ),
                &[("ZEPHON_FW_FIXTURE", "{}"), ("ZEPHON_FW_SCRIPT", script)],
            );
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                matches!(out.status.code(), Some(95 | 97)),
                "{script}: {out:?}"
            );
            assert!(!stdout.contains("ran"), "{script} ran");
        }
    }

    /// Stand-ins for every NetSecurity cmdlet the scripts use, over the rules in the
    /// `ZEPHON_FW_FIXTURE` JSON. They print each change as a `##call` JSON line and never reach
    /// the real firewall: module autoloading is off and NetSecurity is not loaded, so a cmdlet
    /// without a stub is not found, and before anything runs every command name in
    /// `ZEPHON_FW_SCRIPT` (the script under test) must be a stub, a function the script
    /// defines, or one of a few built-ins. .NET is closed the same way: only the types and
    /// methods the scripts use, no `$ExecutionContext` and no `#Requires`, so neither a
    /// scriptblock, reflection nor the firewall's COM or CIM classes can be reached.
    #[cfg(windows)]
    const STUBS: &str = r##"
$ErrorActionPreference = 'Stop'
Import-Module Microsoft.PowerShell.Utility
$PSModuleAutoloadingPreference = 'None'
if (Get-Module NetSecurity) { [Console]::Error.WriteLine('harness: NetSecurity is loaded'); exit 98 }
$ZfwStubs = 'Get-NetFirewallApplicationFilter', 'Get-NetFirewallRule', 'Get-NetFirewallPortFilter',
    'Get-NetFirewallProfile', 'Remove-NetFirewallRule', 'New-NetFirewallRule'
$ZfwBuiltins = 'Where-Object', 'ForEach-Object', 'Out-Null', 'ConvertTo-Json', 'Select-Object', 'Sort-Object'
$ZfwParseErrors = $null
$ZfwAst = [Management.Automation.Language.Parser]::ParseInput($env:ZEPHON_FW_SCRIPT, [ref]$null, [ref]$ZfwParseErrors)
if ($ZfwParseErrors) { [Console]::Error.WriteLine("harness: parse error: $($ZfwParseErrors[0])"); exit 96 }
$ZfwOwn = @($ZfwAst.FindAll({ $args[0] -is [Management.Automation.Language.FunctionDefinitionAst] }, $true) | ForEach-Object { $_.Name })
foreach ($c in $ZfwAst.FindAll({ $args[0] -is [Management.Automation.Language.CommandAst] }, $true)) {
    $n = $c.GetCommandName()
    if (-not $n -or ($ZfwStubs + $ZfwBuiltins + $ZfwOwn) -notcontains $n) {
        [Console]::Error.WriteLine("harness: command without a stub: $($c.Extent.Text)"); exit 97
    }
}
$ZfwTypes = 'int', 'string', 'Text.Encoding', 'Convert', 'Console', 'Management.Automation.WildcardPattern'
$ZfwMethods = 'GetString', 'FromBase64String', 'Escape', 'WriteLine'
$ZfwDotNet = $ZfwAst.FindAll({
    $a = $args[0]
    (($a -is [Management.Automation.Language.TypeExpressionAst] -or $a -is [Management.Automation.Language.TypeConstraintAst]) -and
        $ZfwTypes -notcontains $a.TypeName.FullName) -or
    ($a -is [Management.Automation.Language.InvokeMemberExpressionAst] -and $ZfwMethods -notcontains $a.Member.Value) -or
    ($a -is [Management.Automation.Language.VariableExpressionAst] -and $a.VariablePath.UserPath -eq 'ExecutionContext')
}, $true)
if ($ZfwDotNet -or $ZfwAst.ScriptRequirements) {
    [Console]::Error.WriteLine("harness: .NET use without a stub: $(@($ZfwDotNet)[0].Extent.Text)"); exit 95
}

enum ZfwDirection { Inbound = 1; Outbound = 2 }
enum ZfwAction { NotConfigured = 0; Allow = 2; Block = 4 }
enum ZfwEnabled { True = 1; False = 2 }
enum ZfwGpoBoolean { False = 0; True = 1; NotConfigured = 2 }

$ZfwFx = $env:ZEPHON_FW_FIXTURE | ConvertFrom-Json
$global:ZfwRules = [Collections.ArrayList]@()
function ZfwAddRule($r) {
    $rule = [pscustomobject]@{ Name = $r.id; DisplayName = $r.name; Program = $r.program; Protocol = $r.protocol
        Direction = [ZfwDirection]$r.direction; Action = [ZfwAction]$r.action; Enabled = [ZfwEnabled]$r.enabled }
    [void]$global:ZfwRules.Add($rule)
    $rule
}
foreach ($r in $ZfwFx.rules) { [void](ZfwAddRule $r) }
function ZfwLog($o) {
    $j = $o | ConvertTo-Json -Compress -Depth 4
    [Console]::Out.WriteLine('##call ' + [regex]::Replace($j, '[^\x00-\x7f]', { param($m) '\u{0:x4}' -f [int][char]$m.Value }))
}
function ZfwSelect($DisplayName, $Name) {
    $global:ZfwRules | Where-Object { (-not $DisplayName -or $DisplayName -contains $_.DisplayName) -and (-not $Name -or $Name -contains $_.Name) }
}

function Get-NetFirewallProfile {
    [CmdletBinding()] param()
    if ($ZfwFx.fail -eq 'profile') { Write-Error -Message 'stub: profile query failed' -Category PermissionDenied; return }
    foreach ($p in $ZfwFx.profiles) { [pscustomobject]@{ Enabled = [ZfwGpoBoolean]$p } }
}
function Get-NetFirewallApplicationFilter {
    [CmdletBinding()] param([string[]]$Program)
    if ($ZfwFx.fail -eq 'filter') { Write-Error -Message 'stub: filter query failed' -Category PermissionDenied; return }
    $hit = $false
    foreach ($r in @($global:ZfwRules)) {
        if (-not $PSBoundParameters.ContainsKey('Program') -or @($Program | Where-Object { $r.Program -like $_ }).Count) {
            $hit = $true
            [pscustomobject]@{ Program = $r.Program; ZfwRule = $r.Name }
        }
    }
    if (-not $hit) { Write-Error -Message "stub: no filter for $Program" -Category ObjectNotFound }
}
function Get-NetFirewallRule {
    [CmdletBinding()] param([Parameter(ValueFromPipeline = $true)]$AssociatedNetFirewallApplicationFilter, [string[]]$DisplayName, [string[]]$Name)
    process {
        if ($MyInvocation.ExpectingInput) { ZfwSelect -Name $AssociatedNetFirewallApplicationFilter.ZfwRule }
        else { ZfwSelect $DisplayName $Name }
    }
}
function Get-NetFirewallPortFilter {
    [CmdletBinding()] param([Parameter(ValueFromPipeline = $true)]$AssociatedNetFirewallRule)
    process { [pscustomobject]@{ Protocol = $AssociatedNetFirewallRule.Protocol } }
}
function Remove-NetFirewallRule {
    [CmdletBinding()] param([Parameter(ValueFromPipeline = $true)]$InputObject, [string[]]$DisplayName, [string[]]$Name)
    process {
        $targets = if ($MyInvocation.ExpectingInput) { @($InputObject) } else { @(ZfwSelect $DisplayName $Name) }
        foreach ($t in $targets) {
            ZfwLog @{ cmd = 'remove'; rule = $t.Name }
            $global:ZfwRules = [Collections.ArrayList]@($global:ZfwRules | Where-Object { $_.Name -ne $t.Name })
        }
    }
}
function New-NetFirewallRule {
    [CmdletBinding()] param([string]$DisplayName, [string]$Name, [string]$Description, [string]$Group,
        [string]$Direction = 'Inbound', [string]$Program, [string]$Action = 'Allow', [string[]]$Profile,
        [string]$Enabled = 'True', [string[]]$Protocol, [string[]]$LocalPort, [string[]]$RemotePort,
        [string[]]$LocalAddress, [string[]]$RemoteAddress, [string[]]$InterfaceType, [string]$Service,
        [string]$Package, [string]$EdgeTraversalPolicy, [string]$PolicyStore)
    if ($ZfwFx.fail -eq 'new') { Write-Error -Message 'stub: rule not created' -Category PermissionDenied; return }
    $params = @{}
    foreach ($k in $PSBoundParameters.Keys) {
        if ([Management.Automation.PSCmdlet]::CommonParameters -notcontains $k) { $params[$k] = $PSBoundParameters[$k] -join ',' }
    }
    ZfwLog @{ cmd = 'new'; params = $params }
    $protocol = if ($Protocol) { $Protocol -join ',' } else { 'Any' }
    ZfwAddRule @{ id = 'new'; name = $DisplayName; program = $Program; protocol = $protocol
        direction = $Direction; action = $Action; enabled = $Enabled }
}
# The default, so the script's own error handling is what gets tested.
$ErrorActionPreference = 'Continue'
"##;

    /// What the stubbed run of a script did.
    #[cfg(windows)]
    struct StubRun {
        code: Option<i32>,
        /// The `##call` records, in order.
        calls: Vec<serde_json::Value>,
        /// Everything else on stdout.
        stdout: String,
        stderr: String,
    }

    #[cfg(windows)]
    impl StubRun {
        fn removed(&self) -> std::collections::BTreeSet<&str> {
            self.calls
                .iter()
                .filter(|c| c["cmd"] == "remove")
                .map(|c| c["rule"].as_str().unwrap())
                .collect()
        }

        fn created(&self) -> Vec<&serde_json::Map<String, serde_json::Value>> {
            self.calls
                .iter()
                .filter(|c| c["cmd"] == "new")
                .map(|c| c["params"].as_object().unwrap())
                .collect()
        }
    }

    /// Runs a generated script against [`STUBS`] with `rules` (from [`fw_rule`]), the profiles'
    /// `Enabled` values, and an optional stub to fail (`profile`, `filter` or `new`).
    #[cfg(windows)]
    fn run_stubbed(
        script: &str,
        rules: &[serde_json::Value],
        profiles: &[&str],
        fail: Option<&str>,
    ) -> StubRun {
        let fixture = serde_json::json!({ "rules": rules, "profiles": profiles, "fail": fail });
        let out = powershell(
            &format!("{STUBS}\n{script}"),
            &[
                ("ZEPHON_FW_FIXTURE", &fixture.to_string()),
                ("ZEPHON_FW_SCRIPT", script),
            ],
        );
        let stdout = String::from_utf8(out.stdout).unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let code = out.status.code();
        assert!(
            !matches!(code, Some(95..=98)),
            "harness refused the script: {stderr}"
        );
        let (mut calls, mut rest) = (Vec::new(), String::new());
        for line in stdout.lines() {
            match line.strip_prefix("##call ") {
                Some(json) => calls.push(serde_json::from_str(json).unwrap()),
                None => rest.push_str(line),
            }
        }
        StubRun {
            code,
            calls,
            stdout: rest,
            stderr,
        }
    }

    #[cfg(windows)]
    fn fw_rule(
        id: &str,
        program: &str,
        name: &str,
        direction: &str,
        action: &str,
        enabled: bool,
        protocol: &str,
    ) -> serde_json::Value {
        let enabled = if enabled { "True" } else { "False" };
        serde_json::json!({ "id": id, "program": program, "name": name, "direction": direction,
            "action": action, "enabled": enabled, "protocol": protocol })
    }

    /// This exe: quotes, a subexpression and the wildcard characters `[`, `]` and `*`.
    #[cfg(windows)]
    const OWN: &str = "C:\\Users\\O'Neil\\a\u{2019}b $(x)\\[z] build*\\zephon.exe";
    /// Another copy, which `OWN` matches as an unescaped wildcard pattern (`[z]` is `z`, `*`
    /// is empty).
    #[cfg(windows)]
    const OTHER: &str = "C:\\Users\\O'Neil\\a\u{2019}b $(x)\\z build\\zephon.exe";

    #[cfg(windows)]
    #[test]
    fn install_replaces_only_this_exes_inbound_block_and_old_allow_rules() {
        let (name, legacy) = (rule_name(OWN), netutil::firewall::rule_name(OWN));
        // Nyx names rules by file name only, so the other copy's rule has the same legacy name.
        assert_eq!(legacy, netutil::firewall::rule_name(OTHER));
        let rules = [
            fw_rule(
                "own-block",
                OWN,
                "Block it",
                "Inbound",
                "Block",
                true,
                "Any",
            ),
            fw_rule("own-old", OWN, &name, "Inbound", "Allow", true, "TCP"),
            fw_rule("own-nyx", OWN, &legacy, "Inbound", "Allow", true, "Any"),
            // Windows' "Allow access" prompt rule and an outbound rule with our name: the
            // user's, left alone.
            fw_rule(
                "own-user",
                OWN,
                "zephon.exe",
                "Inbound",
                "Allow",
                true,
                "UDP",
            ),
            fw_rule("own-out", OWN, &name, "Outbound", "Block", true, "Any"),
            fw_rule(
                "other-block",
                OTHER,
                "Block it",
                "Inbound",
                "Block",
                true,
                "Any",
            ),
            fw_rule("other-nyx", OTHER, &legacy, "Inbound", "Allow", true, "Any"),
            fw_rule(
                "unrelated",
                r"C:\Windows\System32\svchost.exe",
                "Core Networking",
                "Inbound",
                "Block",
                true,
                "TCP",
            ),
        ];
        let run = run_stubbed(
            &install_script(OWN, &name, &legacy),
            &rules,
            &["True"; 3],
            None,
        );
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        assert_eq!(
            run.removed(),
            ["own-block", "own-nyx", "own-old"].into_iter().collect()
        );
        let created = run.created();
        assert_eq!(created.len(), 1, "{:?}", run.calls);
        let new = created[0];
        assert_eq!(new["DisplayName"], format!("Zephon - {OWN}"));
        assert_eq!(new["Program"], OWN);
        for (key, value) in [
            ("Direction", "Inbound"),
            ("Action", "Allow"),
            ("Profile", "Any"),
        ] {
            let got = new.get(key).and_then(|v| v.as_str()).unwrap_or_default();
            assert!(got.eq_ignore_ascii_case(value), "{key}: {got:?}");
        }
        // Not disabled, and not limited to a protocol, port, address or store.
        let enabled = new.get("Enabled").and_then(|v| v.as_str());
        assert!(
            enabled.is_none_or(|e| e.eq_ignore_ascii_case("True")),
            "{enabled:?}"
        );
        let allowed = [
            "DisplayName",
            "Program",
            "Direction",
            "Action",
            "Profile",
            "Enabled",
            "Description",
            "Group",
        ];
        for key in new.keys() {
            assert!(allowed.contains(&key.as_str()), "unexpected -{key}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn check_reads_only_this_exes_rules_from_the_firewall() {
        let check = |rules: &[serde_json::Value], profiles: &[&str]| {
            let run = run_stubbed(&check_script(OWN), rules, profiles, None);
            assert_eq!(run.code, Some(0), "{}", run.stderr);
            assert!(
                run.calls.is_empty(),
                "the check changed rules: {:?}",
                run.calls
            );
            parse_report(&run.stdout).unwrap()
        };
        let other = [
            fw_rule("other-udp", OTHER, "x", "Inbound", "Allow", true, "UDP"),
            fw_rule("other-block", OTHER, "x", "Inbound", "Block", true, "Any"),
        ];
        let mut rules = vec![
            fw_rule("own-tcp", OWN, "x", "Inbound", "Allow", true, "TCP"),
            fw_rule("own-udp-off", OWN, "x", "Inbound", "Allow", false, "UDP"),
            fw_rule("own-udp-out", OWN, "x", "Outbound", "Allow", true, "UDP"),
        ];
        rules.extend(other.clone());
        assert_eq!(
            check(&rules, &["False", "True", "False"]),
            Report {
                firewall_enabled: true,
                tcp: true,
                udp: false,
                blocked: false
            }
        );
        let rules = [
            fw_rule("own-any", OWN, "x", "Inbound", "Allow", true, "Any"),
            fw_rule("own-block", OWN, "x", "Inbound", "Block", true, "UDP"),
        ];
        assert_eq!(
            check(&rules, &["False"; 3]),
            Report {
                firewall_enabled: false,
                tcp: true,
                udp: true,
                blocked: true
            }
        );
        // No rule for this exe at all is the normal first-run case, not a query error.
        assert_eq!(
            check(&other, &["True"; 3]),
            Report {
                firewall_enabled: true,
                ..Default::default()
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn scripts_fail_loudly_when_a_firewall_query_fails() {
        let rules = [fw_rule(
            "own-tcp", OWN, "x", "Inbound", "Allow", true, "TCP",
        )];
        for fail in ["profile", "filter"] {
            let run = run_stubbed(&check_script(OWN), &rules, &["True"; 3], Some(fail));
            assert_eq!(run.code, Some(1), "{fail}: {}", run.stdout);
            assert!(run.stdout.is_empty(), "{fail}: {}", run.stdout);
            assert!(
                run.stderr.contains(&format!("stub: {fail} query failed")),
                "{fail}: {}",
                run.stderr
            );
        }
        let (name, legacy) = (rule_name(OWN), netutil::firewall::rule_name(OWN));
        let run = run_stubbed(
            &install_script(OWN, &name, &legacy),
            &rules,
            &["True"; 3],
            Some("new"),
        );
        assert_eq!(run.code, Some(1), "{:?}", run.calls);
    }
}
