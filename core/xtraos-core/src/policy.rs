//! Policy engine (Phase 1 M5, ADR 0017; classes and defaults from docs/05-security-model.md).
//! Pure decision logic: `safety.rs` gathers the facts about an action (`Facts`), this module
//! decides allow / confirm / deny. The agent cannot skip it: the core evaluates it before
//! every OS action.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Decision {
    Allow,
    Confirm,
    Deny,
}

impl Decision {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "allow" => Some(Decision::Allow),
            "confirm" => Some(Decision::Confirm),
            "deny" => Some(Decision::Deny),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    Observe,
    Input,
    Launch,
    Close,
    FileSend,
    MessageSend,
    Delete,
    Purchase,
    SystemSettings,
    Elevated,
    CredentialEntry,
    /// Creating, copying, moving, renaming files through file skills (P2-M3).
    FileChange,
    /// Closing a window the agent did not open (P2-M4, ADR 0028).
    CloseUserWindow,
}

impl Class {
    pub const ALL: [Class; 13] = [
        Class::Observe,
        Class::Input,
        Class::Launch,
        Class::Close,
        Class::FileSend,
        Class::MessageSend,
        Class::Delete,
        Class::Purchase,
        Class::SystemSettings,
        Class::Elevated,
        Class::CredentialEntry,
        Class::FileChange,
        Class::CloseUserWindow,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Class::Observe => "observe",
            Class::Input => "input",
            Class::Launch => "launch",
            Class::Close => "close",
            Class::FileSend => "file_send",
            Class::MessageSend => "message_send",
            Class::Delete => "delete",
            Class::Purchase => "purchase",
            Class::SystemSettings => "system_settings",
            Class::Elevated => "elevated",
            Class::CredentialEntry => "credential_entry",
            Class::FileChange => "file_change",
            Class::CloseUserWindow => "close_user_window",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Class::ALL.into_iter().find(|c| c.name() == s)
    }

    fn default_decision(self) -> Decision {
        match self {
            Class::Observe | Class::Input | Class::Launch | Class::Close | Class::FileChange => {
                Decision::Allow
            }
            Class::MessageSend
            | Class::FileSend
            | Class::Delete
            | Class::SystemSettings
            | Class::Elevated
            | Class::CloseUserWindow => Decision::Confirm,
            Class::Purchase | Class::CredentialEntry => Decision::Deny,
        }
    }
}

/// Facts about one action, gathered by safety.rs (no OS calls here).
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub method: String,
    /// Windows the action touches: (process name, title), lowercase.
    pub windows: Vec<(String, String)>,
    /// Name of the element acted on (ref target, or the UIA element under a real click).
    pub element: Option<String>,
    /// The target (or focused) element is a password field.
    pub password: bool,
    /// `type` with submit, or Enter / Ctrl+Enter among `press_keys`.
    pub enter: bool,
    /// Delete / Shift+Delete among `press_keys`.
    pub delete_key: bool,
    /// `launch_app` with elevated: true.
    pub elevated: bool,
    /// `drag`: process of the drop window (lowercase).
    pub drop_process: Option<(String, String)>,
    pub drag_from_explorer: bool,
    /// The action touches one of XtraOS's own windows.
    pub own_window: bool,
    /// Class declared by a skill (P2-M3), e.g. `delete` for files.delete.
    pub skill_class: Option<Class>,
    /// close_window on a window the agent did not open (ADR 0028).
    pub closes_user_window: bool,
}

const MESSAGING_PROCESSES: &[&str] = &[
    "whatsapp.exe",
    "whatsapp.root.exe",
    "telegram.exe",
    "slack.exe",
    "ms-teams.exe",
    "teams.exe",
    "discord.exe",
    "signal.exe",
    "outlook.exe",
    "olk.exe",
    "thunderbird.exe",
    "skype.exe",
    "zoom.exe",
    "messenger.exe",
];
const MESSAGING_TITLES: &[&str] = &[
    "whatsapp",
    "telegram",
    "slack",
    "gmail",
    "outlook",
    "microsoft teams",
    "discord",
    "messenger",
    "signal",
    "instagram",
    "linkedin",
];
const BROWSERS: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "opera.exe",
    "vivaldi.exe",
    "arc.exe",
];
const SETTINGS_PROCESSES: &[&str] = &[
    "systemsettings.exe",
    "mmc.exe",
    "regedit.exe",
    "control.exe",
    "gpedit.exe",
    "msconfig.exe",
];
const SETTINGS_TITLES: &[&str] = &["control panel", "registry editor", "group policy"];
/// Password managers + Windows security/credential UIs (docs/05 default deny list).
/// XtraOS's own processes (viewer/editor, admin helper, other cores): never driven by the
/// agent, whatever policy.json says (ADR 0036).
pub const XTRAOS_PROCESS: &str = "xtraos-core.exe";
const DENY_PROCESSES: &[&str] = &[
    XTRAOS_PROCESS,
    "1password.exe",
    "keepass.exe",
    "keepassxc.exe",
    "bitwarden.exe",
    "lastpass.exe",
    "dashlane.exe",
    "enpass.exe",
    "roboform.exe",
    "nordpass.exe",
    "keeper.exe",
    "keeperpasswordmanager.exe",
    "sechealthui.exe",
    "credentialuibroker.exe",
];
const DENY_TITLES: &[&str] = &["credential manager", "windows security"];

const SEND_WORDS: &[&str] = &[
    "send",
    "send message",
    "send now",
    "send email",
    "post",
    "reply",
    "reply all",
    "forward",
    "submit",
    "tweet",
    "share",
    "publish",
];
const PURCHASE_WORDS: &[&str] = &[
    "buy",
    "buy now",
    "place order",
    "place your order",
    "pay",
    "pay now",
    "checkout",
    "check out",
    "proceed to checkout",
    "purchase",
    "complete purchase",
    "confirm order",
    "confirm purchase",
    "subscribe",
];
const DELETE_WORDS: &[&str] = &[
    "delete",
    "delete permanently",
    "permanently delete",
    "remove",
    "move to trash",
    "move to recycle bin",
    "empty recycle bin",
    "empty trash",
    "uninstall",
    "erase",
    "format",
];

// Localised words (ADR 0034): Hindi (Devanagari + common romanised forms), Marathi, Spanish,
// French, German, Portuguese. Same exact-name rule as the English lists.
const SEND_WORDS_INTL: &[&str] = &[
    "भेजें",
    "भेजो",
    "भेज दें",
    "भेजिए",
    "संदेश भेजें",
    "जमा करें",
    "उत्तर दें",
    "जवाब दें",
    "साझा करें",
    "शेयर करें",
    "पोस्ट करें",
    "प्रकाशित करें",
    "आगे भेजें",
    "bhejo",
    "bhejein",
    "bheje",
    "bhej do",
    "send karo",
    "पाठवा",
    "उत्तर द्या",
    "शेअर करा",
    "सबमिट करा",
    "enviar",
    "enviar mensaje",
    "responder",
    "responder a todos",
    "reenviar",
    "publicar",
    "compartir",
    "envoyer",
    "répondre",
    "répondre à tous",
    "transférer",
    "publier",
    "partager",
    "soumettre",
    "senden",
    "absenden",
    "abschicken",
    "antworten",
    "allen antworten",
    "weiterleiten",
    "veröffentlichen",
    "teilen",
    "encaminhar",
    "compartilhar",
    "partilhar",
    "enviar mensagem",
];
const PURCHASE_WORDS_INTL: &[&str] = &[
    "खरीदें",
    "अभी खरीदें",
    "भुगतान करें",
    "ऑर्डर करें",
    "ऑर्डर दें",
    "ऑर्डर प्लेस करें",
    "kharido",
    "kharidein",
    "abhi kharidein",
    "खरेदी करा",
    "आता खरेदी करा",
    "पेमेंट करा",
    "ऑर्डर करा",
    "comprar",
    "comprar ahora",
    "pagar",
    "pagar ahora",
    "realizar pedido",
    "finalizar compra",
    "tramitar pedido",
    "suscribirse",
    "acheter",
    "acheter maintenant",
    "payer",
    "commander",
    "passer la commande",
    "valider la commande",
    "kaufen",
    "jetzt kaufen",
    "bezahlen",
    "bestellen",
    "jetzt bestellen",
    "zahlungspflichtig bestellen",
    "kostenpflichtig bestellen",
    "zur kasse",
    "abonnieren",
    "comprar agora",
    "fazer pedido",
    "finalizar pedido",
    "assinar",
];
const DELETE_WORDS_INTL: &[&str] = &[
    "हटाएं",
    "हटाएँ",
    "मिटाएं",
    "मिटाएँ",
    "डिलीट करें",
    "स्थायी रूप से हटाएं",
    "hatao",
    "mitao",
    "delete karo",
    "हटवा",
    "काढून टाका",
    "डिलीट करा",
    "eliminar",
    "borrar",
    "eliminar permanentemente",
    "vaciar papelera",
    "desinstalar",
    "supprimer",
    "supprimer définitivement",
    "effacer",
    "vider la corbeille",
    "désinstaller",
    "löschen",
    "endgültig löschen",
    "entfernen",
    "papierkorb leeren",
    "deinstallieren",
    "excluir",
    "apagar",
    "excluir permanentemente",
    "esvaziar lixeira",
    "remover",
];

/// True if the element name *is* one of the words (ignoring case, punctuation, a trailing
/// count like "Send (2)"), so "Sender list" or "Paypal settings" do not match. Combining marks
/// (Devanagari vowel signs) are kept: they are part of the word.
pub fn name_is(name: &str, words: &[&str]) -> bool {
    let clean: String = name
        .to_lowercase()
        .chars()
        .map(|c| {
            let mark_or_letter = !c.is_ascii()
                && !c.is_whitespace()
                && !matches!(
                    c,
                    '।' | '॥'
                        | '“'
                        | '”'
                        | '‘'
                        | '’'
                        | '…'
                        | '–'
                        | '—'
                        | '«'
                        | '»'
                        | '¡'
                        | '¿'
                );
            if c.is_alphanumeric() || c == ' ' || mark_or_letter {
                c
            } else {
                ' '
            }
        })
        .collect();
    let clean = clean
        .split_whitespace()
        .filter(|w| !w.chars().all(|c| c.is_ascii_digit()))
        .collect::<Vec<_>>()
        .join(" ");
    words.iter().any(|w| clean == *w)
}

pub fn is_messaging(p: &str, title: &str) -> bool {
    MESSAGING_PROCESSES.contains(&p)
        || (BROWSERS.contains(&p) && MESSAGING_TITLES.iter().any(|t| title.contains(t)))
}

fn method_class(m: &str) -> Class {
    match m {
        "observe" | "find" | "read_text" | "screenshot" | "list_windows" | "ocr" | "wait_for"
        | "frame_diff" => Class::Observe,
        m if m.starts_with("run_skill:")
            || m == "app_map"
            || m == "elevated_helper"
            || m == "record_session"
            || m == "volume:get"
            || m == "system:read" =>
        {
            Class::Observe
        }
        m if m.starts_with("browser:") => {
            if crate::browser::is_observe(&m[8..]) {
                Class::Observe
            } else {
                Class::Input
            }
        }
        "launch_app" => Class::Launch,
        "cleanup" => Class::Close,
        "close_window" => Class::Close,
        _ => Class::Input,
    }
}

const INPUT_METHODS: &[&str] = &[
    "click",
    "type",
    "press_keys",
    "scroll",
    "drag",
    "mouse",
    "set_value",
    "focus_window",
    "window_bounds",
    "close_window",
];

/// All classes an action falls into, with a short reason each.
pub fn classify(f: &Facts) -> Vec<(Class, String)> {
    let mut out = vec![(method_class(&f.method), f.method.clone())];
    let m = f.method.as_str();
    let acts = INPUT_METHODS.contains(&m);
    if let Some(name) = &f.element {
        if m == "click" || m == "set_value" || m == "browser:click" {
            if name_is(name, SEND_WORDS) || name_is(name, SEND_WORDS_INTL) {
                out.push((Class::MessageSend, format!("\"{name}\"")));
            }
            if name_is(name, PURCHASE_WORDS) || name_is(name, PURCHASE_WORDS_INTL) {
                out.push((Class::Purchase, format!("\"{name}\"")));
            }
            if name_is(name, DELETE_WORDS) || name_is(name, DELETE_WORDS_INTL) {
                out.push((Class::Delete, format!("\"{name}\"")));
            }
        }
    }
    for (p, title) in &f.windows {
        if f.enter && is_messaging(p, title) {
            out.push((Class::MessageSend, "Enter in a messaging app".into()));
        }
        if f.delete_key && p == "explorer.exe" {
            out.push((Class::Delete, "Delete key in File Explorer".into()));
        }
        if acts
            && (SETTINGS_PROCESSES.contains(&p.as_str())
                || SETTINGS_TITLES.iter().any(|t| title.contains(t)))
        {
            out.push((Class::SystemSettings, "system settings window".into()));
        }
    }
    if let Some((p, title)) = &f.drop_process {
        if f.drag_from_explorer && is_messaging(p, title) {
            out.push((Class::FileSend, "file dragged into a messaging app".into()));
        }
    }
    if f.elevated {
        out.push((Class::Elevated, "administrator (UAC) start".into()));
    }
    if f.closes_user_window {
        out.push((Class::CloseUserWindow, "a window you opened".into()));
    }
    if let Some(c) = f.skill_class {
        out.push((c, f.method.clone()));
    }
    if f.password
        && matches!(
            m,
            "type" | "set_value" | "press_keys" | "browser:type" | "browser:press_key"
        )
    {
        out.push((Class::CredentialEntry, "password field".into()));
    }
    out
}

/// A loaded policy (defaults + user file).
#[derive(Clone, Debug)]
pub struct Policy {
    pub classes: HashMap<Class, Decision>,
    pub apps: HashMap<String, Decision>,
    pub rate_per_minute: u32,
    /// Runaway guard (ADR 0038): actions per task before the user is asked to continue.
    pub max_actions_per_task: u32,
    /// A pause this long ends a task.
    pub task_idle_minutes: u32,
    /// Source file (if any) and a short hash of its content, for the action log.
    pub source: Option<(PathBuf, String)>,
    pub warnings: Vec<String>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            classes: Class::ALL
                .into_iter()
                .map(|c| (c, c.default_decision()))
                .collect(),
            apps: DENY_PROCESSES
                .iter()
                .map(|p| (p.to_string(), Decision::Deny))
                .collect(),
            rate_per_minute: 600,
            max_actions_per_task: 2000,
            task_idle_minutes: 10,
            source: None,
            warnings: Vec::new(),
        }
    }
}

fn fnv(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

impl Policy {
    /// Defaults overlaid with a JSON document. Unknown keys are reported, not fatal.
    pub fn from_json(text: &str) -> Policy {
        let mut p = Policy::default();
        let v: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                p.warnings
                    .push(format!("policy.json ignored (invalid JSON: {e})"));
                return p;
            }
        };
        if let Some(cs) = v.get("classes").and_then(Value::as_object) {
            for (k, d) in cs {
                match (Class::parse(k), d.as_str().and_then(Decision::parse)) {
                    (Some(c), Some(d)) => {
                        p.classes.insert(c, d);
                    }
                    _ => p.warnings.push(format!("classes.{k}: ignored")),
                }
            }
        }
        if let Some(apps) = v.get("apps").and_then(Value::as_object) {
            for (k, d) in apps {
                match d.as_str().and_then(Decision::parse) {
                    Some(d) => {
                        p.apps.insert(k.to_lowercase(), d);
                    }
                    None => p.warnings.push(format!("apps.{k}: ignored")),
                }
            }
        }
        if let Some(r) = v.get("rate_per_minute").and_then(Value::as_u64) {
            p.rate_per_minute = r.clamp(10, 10_000) as u32;
        }
        if let Some(r) = v.get("max_actions_per_task").and_then(Value::as_u64) {
            p.max_actions_per_task = r.clamp(5, 1_000_000) as u32;
        }
        if let Some(r) = v.get("task_idle_minutes").and_then(Value::as_u64) {
            p.task_idle_minutes = r.clamp(1, 1440) as u32;
        }
        p.enforce_hard_rules();
        p
    }

    /// Rules the user cannot loosen (ADR 0017, CLAUDE.md §4).
    fn enforce_hard_rules(&mut self) {
        if self.classes.get(&Class::CredentialEntry) != Some(&Decision::Deny) {
            self.warnings.push("credential_entry is always deny".into());
            self.classes.insert(Class::CredentialEntry, Decision::Deny);
        }
        if self.apps.get(XTRAOS_PROCESS) != Some(&Decision::Deny) {
            self.warnings
                .push("xtraos-core.exe windows are always off limits".into());
            self.apps.insert(XTRAOS_PROCESS.into(), Decision::Deny);
        }
        if self.classes.get(&Class::Purchase) == Some(&Decision::Allow) {
            self.warnings.push("purchase can be at most confirm".into());
            self.classes.insert(Class::Purchase, Decision::Confirm);
        }
    }

    /// Loads `path` (missing file = defaults).
    pub fn load(path: &Path) -> Policy {
        match std::fs::read_to_string(path) {
            Ok(t) => {
                let mut p = Policy::from_json(&t);
                p.source = Some((path.to_path_buf(), fnv(&t)));
                p
            }
            Err(_) => Policy::default(),
        }
    }

    /// The final decision and the reason that drove it.
    pub fn decide(&self, f: &Facts) -> (Decision, String) {
        if f.own_window {
            return (
                Decision::Deny,
                "XtraOS's own windows are for the user only".into(),
            );
        }
        let mut best = (Decision::Allow, f.method.clone());
        for (p, title) in f.windows.iter().chain(f.drop_process.iter()) {
            let d = self.apps.get(p).copied().unwrap_or_else(|| {
                if DENY_TITLES.iter().any(|t| title.contains(t)) {
                    Decision::Deny
                } else {
                    Decision::Allow
                }
            });
            if d > best.0 {
                best = (d, format!("app rule for {p}"));
            }
        }
        for (c, why) in classify(f) {
            let d = self.classes.get(&c).copied().unwrap_or(Decision::Confirm);
            if d > best.0 {
                best = (d, format!("{} ({why})", c.name()));
            }
        }
        best
    }
}

/// Sliding one-minute window rate limiter.
#[derive(Default)]
pub struct RateLimiter {
    times: std::collections::VecDeque<u64>,
}

impl RateLimiter {
    /// `now_ms` is any monotonic millisecond clock. Returns false when over the limit.
    pub fn admit(&mut self, now_ms: u64, per_minute: u32) -> bool {
        while self
            .times
            .front()
            .is_some_and(|&t| now_ms.saturating_sub(t) >= 60_000)
        {
            self.times.pop_front();
        }
        if self.times.len() >= per_minute as usize {
            return false;
        }
        self.times.push_back(now_ms);
        true
    }
}

/// Max actions per task (ADR 0038). A task is a run of actions without a pause of
/// `idle_ms`. At the limit the user is asked; after "Stop" actions are refused until the
/// agent pauses (or the user resumes from the tray).
#[derive(Default)]
pub struct TaskLimiter {
    count: u32,
    last_ms: Option<u64>,
    stopped: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TaskCheck {
    Go,
    Ask(u32),
    Refused,
}

impl TaskLimiter {
    pub fn check(&mut self, now_ms: u64, max: u32, idle_ms: u64) -> TaskCheck {
        if self
            .last_ms
            .is_some_and(|l| now_ms.saturating_sub(l) >= idle_ms)
        {
            self.count = 0;
            self.stopped = false;
        }
        self.last_ms = Some(now_ms);
        if self.stopped {
            return TaskCheck::Refused;
        }
        if self.count >= max {
            return TaskCheck::Ask(self.count);
        }
        self.count += 1;
        TaskCheck::Go
    }
    /// The user said Continue: a fresh allowance, this action counts.
    pub fn continue_task(&mut self) {
        self.count = 1;
    }
    pub fn stop(&mut self) {
        self.stopped = true;
    }
    pub fn reset(&mut self) {
        *self = TaskLimiter::default();
    }
}

/// Cached policy that reloads when the file's modification time changes.
pub struct Store {
    path: PathBuf,
    mtime: Option<SystemTime>,
    pub policy: Policy,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        let policy = Policy::load(&path);
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        Store {
            path,
            mtime,
            policy,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A changed file, loaded but NOT applied (the user must approve it first, ADR 0026).
    /// Remembers the change so it is asked about once.
    pub fn changed(&mut self) -> Option<Policy> {
        let now = std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok();
        if now == self.mtime {
            return None;
        }
        self.mtime = now;
        let cand = Policy::load(&self.path);
        if cand.source.as_ref().map(|s| &s.1) == self.policy.source.as_ref().map(|s| &s.1) {
            return None; // touched but same content
        }
        Some(cand)
    }

    pub fn apply(&mut self, p: Policy) {
        self.policy = p;
    }
}

/// Human-readable differences between two policies (for the approval dialog and the log).
pub fn summary(old: &Policy, new: &Policy) -> Vec<String> {
    let name = |d: Option<&Decision>| match d {
        Some(Decision::Allow) => "allow",
        Some(Decision::Confirm) => "confirm",
        Some(Decision::Deny) => "deny",
        None => "(default)",
    };
    let mut out = Vec::new();
    for c in Class::ALL {
        let (a, b) = (old.classes.get(&c), new.classes.get(&c));
        if a != b {
            out.push(format!("{}: {} → {}", c.name(), name(a), name(b)));
        }
    }
    let mut apps: Vec<&String> = old.apps.keys().chain(new.apps.keys()).collect();
    apps.sort();
    apps.dedup();
    for a in apps {
        let (x, y) = (old.apps.get(a), new.apps.get(a));
        if x != y {
            out.push(format!("app {a}: {} → {}", name(x), name(y)));
        }
    }
    if old.max_actions_per_task != new.max_actions_per_task {
        out.push(format!(
            "max_actions_per_task: {} → {}",
            old.max_actions_per_task, new.max_actions_per_task
        ));
    }
    if old.task_idle_minutes != new.task_idle_minutes {
        out.push(format!(
            "task_idle_minutes: {} → {}",
            old.task_idle_minutes, new.task_idle_minutes
        ));
    }
    if old.rate_per_minute != new.rate_per_minute {
        out.push(format!(
            "rate_per_minute: {} → {}",
            old.rate_per_minute, new.rate_per_minute
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(method: &str) -> Facts {
        Facts {
            method: method.into(),
            ..Default::default()
        }
    }

    fn win(p: &str, t: &str) -> Vec<(String, String)> {
        vec![(p.into(), t.into())]
    }

    #[test]
    fn defaults_match_docs05() {
        let p = Policy::default();
        assert_eq!(p.decide(&facts("observe")).0, Decision::Allow);
        assert_eq!(p.decide(&facts("click")).0, Decision::Allow);
        assert_eq!(p.decide(&facts("launch_app")).0, Decision::Allow);
        assert_eq!(p.decide(&facts("close_window")).0, Decision::Allow);
        let mut f = facts("launch_app");
        f.elevated = true;
        assert_eq!(p.decide(&f).0, Decision::Confirm);
    }

    #[test]
    fn send_buy_delete_buttons_are_recognised() {
        let p = Policy::default();
        let mut f = facts("click");
        f.element = Some("Send".into());
        let (d, why) = p.decide(&f);
        assert_eq!(d, Decision::Confirm);
        assert!(why.starts_with("message_send"), "{why}");
        f.element = Some("Send (2)".into());
        assert_eq!(p.decide(&f).0, Decision::Confirm);
        f.element = Some("Place order".into());
        assert_eq!(p.decide(&f).0, Decision::Deny);
        f.element = Some("Move to Recycle Bin".into());
        assert_eq!(p.decide(&f).0, Decision::Confirm);
        // look-alikes are not matched
        for n in [
            "Sender",
            "Paypal settings",
            "Delete history settings",
            "Send feedback form",
        ] {
            f.element = Some(n.into());
            assert_eq!(p.decide(&f).0, Decision::Allow, "{n}");
        }
        // observing a Send button is fine
        let mut o = facts("find");
        o.element = Some("Send".into());
        assert_eq!(p.decide(&o).0, Decision::Allow);
    }

    #[test]
    fn enter_in_messaging_apps_is_message_send() {
        let p = Policy::default();
        let mut f = facts("type");
        f.enter = true;
        f.windows = win("whatsapp.exe", "whatsapp");
        assert_eq!(p.decide(&f).0, Decision::Confirm);
        f.windows = win("chrome.exe", "(3) whatsapp - google chrome");
        assert_eq!(p.decide(&f).0, Decision::Confirm);
        f.windows = win("notepad.exe", "untitled - notepad");
        assert_eq!(p.decide(&f).0, Decision::Allow);
        f.windows = win("chrome.exe", "google - google chrome");
        assert_eq!(
            p.decide(&f).0,
            Decision::Allow,
            "plain browser search is fine"
        );
    }

    #[test]
    fn deny_list_settings_password_and_file_send() {
        let p = Policy::default();
        let mut f = facts("observe");
        f.windows = win("keepassxc.exe", "passwords.kdbx - keepassxc");
        assert_eq!(p.decide(&f).0, Decision::Deny, "even observing");
        f.windows = win("explorer.exe", "credential manager");
        assert_eq!(p.decide(&f).0, Decision::Deny);
        let mut s = facts("click");
        s.windows = win("systemsettings.exe", "settings");
        assert_eq!(p.decide(&s).0, Decision::Confirm);
        let mut so = facts("observe");
        so.windows = win("systemsettings.exe", "settings");
        assert_eq!(p.decide(&so).0, Decision::Allow, "reading settings is fine");
        let mut t = facts("type");
        t.password = true;
        assert_eq!(p.decide(&t).0, Decision::Deny);
        let mut d = facts("drag");
        d.drag_from_explorer = true;
        d.drop_process = Some(("telegram.exe".into(), "telegram".into()));
        assert_eq!(p.decide(&d).0, Decision::Confirm);
        let mut k = facts("press_keys");
        k.delete_key = true;
        k.windows = win("explorer.exe", "documents");
        assert_eq!(p.decide(&k).0, Decision::Confirm);
        let mut own = facts("click");
        own.own_window = true;
        assert_eq!(p.decide(&own).0, Decision::Deny);
    }

    #[test]
    fn user_file_overrides_but_hard_rules_hold() {
        let p = Policy::from_json(
            r#"{"classes": {"message_send": "allow", "credential_entry": "allow", "purchase": "allow",
                "input": "confirm", "bogus": "allow"},
               "apps": {"Testbed.exe": "deny", "keepass.exe": "allow"}, "rate_per_minute": 5}"#,
        );
        assert_eq!(p.classes[&Class::MessageSend], Decision::Allow);
        assert_eq!(p.classes[&Class::CredentialEntry], Decision::Deny);
        assert_eq!(p.classes[&Class::Purchase], Decision::Confirm);
        assert_eq!(p.classes[&Class::Input], Decision::Confirm);
        assert_eq!(p.apps["testbed.exe"], Decision::Deny);
        assert_eq!(
            p.apps["keepass.exe"],
            Decision::Allow,
            "user may allow an app"
        );
        assert_eq!(p.rate_per_minute, 10, "clamped");
        assert!(p.warnings.iter().any(|w| w.contains("bogus")));
        assert!(p.warnings.iter().any(|w| w.contains("credential_entry")));
        let bad = Policy::from_json("{not json");
        assert_eq!(bad.classes[&Class::MessageSend], Decision::Confirm);
        assert!(!bad.warnings.is_empty());
    }

    #[test]
    fn summary_lists_loosened_rules() {
        let old = Policy::default();
        let new = Policy::from_json(
            r#"{"classes": {"message_send": "allow"}, "apps": {"keepass.exe": "allow"}}"#,
        );
        let s = summary(&old, &new);
        assert!(
            s.contains(&"message_send: confirm → allow".to_string()),
            "{s:?}"
        );
        assert!(
            s.contains(&"app keepass.exe: deny → allow".to_string()),
            "{s:?}"
        );
        assert!(summary(&old, &Policy::default()).is_empty());
        assert_eq!(
            Policy::default().classes[&Class::FileChange],
            Decision::Allow
        );
    }

    #[test]
    fn rate_limiter_slides() {
        let mut r = RateLimiter::default();
        for i in 0..3 {
            assert!(r.admit(i, 3));
        }
        assert!(!r.admit(10, 3));
        assert!(r.admit(60_001, 3), "first entry expired");
    }

    #[test]
    fn names_are_matched_as_whole_phrases() {
        assert!(name_is("  SEND! ", SEND_WORDS));
        assert!(name_is("Reply all", SEND_WORDS));
        assert!(!name_is("Send to OneNote settings", SEND_WORDS));
        assert!(name_is("Buy now", PURCHASE_WORDS));
    }

    #[test]
    fn localised_words_match_exactly() {
        for (n, w) in [
            ("भेजें", SEND_WORDS_INTL),
            ("Bhejo", SEND_WORDS_INTL),
            ("पाठवा", SEND_WORDS_INTL),
            ("Enviar", SEND_WORDS_INTL),
            ("Envoyer (2)", SEND_WORDS_INTL),
            ("Senden", SEND_WORDS_INTL),
            ("अभी खरीदें", PURCHASE_WORDS_INTL),
            ("Jetzt kaufen", PURCHASE_WORDS_INTL),
            ("Comprar agora!", PURCHASE_WORDS_INTL),
            ("Passer la commande", PURCHASE_WORDS_INTL),
            ("हटाएं", DELETE_WORDS_INTL),
            ("Löschen", DELETE_WORDS_INTL),
            ("Supprimer définitivement", DELETE_WORDS_INTL),
        ] {
            assert!(name_is(n, w), "{n}");
        }
        // Exact names only: longer labels that merely contain a word do not count.
        assert!(!name_is("भेजें गए संदेश", SEND_WORDS_INTL), "sent-items folder");
        assert!(!name_is("Enviados", SEND_WORDS_INTL));
        assert!(!name_is("Kaufen Sie mehr Speicher", PURCHASE_WORDS_INTL));
        assert!(!name_is("Papierkorb", DELETE_WORDS_INTL));
        // The policy uses them.
        let f = Facts {
            method: "click".into(),
            element: Some("अभी खरीदें".into()),
            ..Default::default()
        };
        assert!(classify(&f).iter().any(|(c, _)| *c == Class::Purchase));
    }

    #[test]
    fn task_limiter_asks_continues_stops_and_resets_after_a_pause() {
        let mut t = TaskLimiter::default();
        for i in 0..3 {
            assert_eq!(t.check(i * 1000, 3, 600_000), TaskCheck::Go);
        }
        assert_eq!(t.check(4000, 3, 600_000), TaskCheck::Ask(3));
        t.continue_task();
        assert_eq!(t.check(5000, 3, 600_000), TaskCheck::Go);
        assert_eq!(t.check(6000, 3, 600_000), TaskCheck::Go);
        assert_eq!(t.check(7000, 3, 600_000), TaskCheck::Ask(3));
        t.stop();
        assert_eq!(
            t.check(8000, 3, 600_000),
            TaskCheck::Refused,
            "stays stopped while busy"
        );
        assert_eq!(
            t.check(8000 + 600_000, 3, 600_000),
            TaskCheck::Go,
            "a pause ends the task"
        );
        let p = Policy::from_json(r#"{"max_actions_per_task": 2, "task_idle_minutes": 0}"#);
        assert_eq!(
            (p.max_actions_per_task, p.task_idle_minutes),
            (5, 1),
            "clamped"
        );
    }
}
