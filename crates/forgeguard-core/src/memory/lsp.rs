//! Optional second opinion on a call's receiver type, asked of a real language
//! server.
//!
//! The static pass in `extract` only records a receiver when the answer is
//! obvious from the file itself; anything that needs type inference across files
//! stays `None`. A language server already did that work for the editor, so when
//! one is installed we ask it — and only when its answer is unambiguous, because
//! a wrong receiver poisons the store's caller/callee joins worse than a missing
//! one does.
//!
//! Everything here is best effort and bounded: a server that is missing, slow,
//! or broken must never fail an index run. Each request carries a timeout, the
//! whole run carries a budget, and a family that misbehaves once is disabled for
//! the rest of the run.

use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

use serde_json::{json, Value};

/// A language server ForgeGuard knows how to drive.
pub struct ServerSpec {
    pub family: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
}

/// Servers we look for, keyed by `LanguageProfile::family()`. Several entries
/// may share a family: the first one found on `PATH` wins, so the preferred
/// implementation comes first.
pub const SERVERS: &[ServerSpec] = &[
    ServerSpec {
        family: "rust",
        command: "rust-analyzer",
        args: &[],
    },
    ServerSpec {
        family: "javascript",
        command: "typescript-language-server",
        args: &["--stdio"],
    },
    ServerSpec {
        family: "javascript",
        command: "vtsls",
        args: &["--stdio"],
    },
    ServerSpec {
        family: "python",
        command: "pyright-langserver",
        args: &["--stdio"],
    },
    ServerSpec {
        family: "python",
        command: "basedpyright-langserver",
        args: &["--stdio"],
    },
    ServerSpec {
        family: "python",
        command: "pylsp",
        args: &[],
    },
    ServerSpec {
        family: "go",
        command: "gopls",
        args: &[],
    },
    ServerSpec {
        family: "c",
        command: "clangd",
        args: &[],
    },
    ServerSpec {
        family: "cpp",
        command: "clangd",
        args: &[],
    },
    ServerSpec {
        family: "java",
        command: "jdtls",
        args: &[],
    },
    ServerSpec {
        family: "kotlin",
        command: "kotlin-language-server",
        args: &[],
    },
    ServerSpec {
        family: "csharp",
        command: "OmniSharp",
        args: &["-lsp"],
    },
    ServerSpec {
        family: "csharp",
        command: "csharp-ls",
        args: &[],
    },
    ServerSpec {
        family: "php",
        command: "phpactor",
        args: &["language-server"],
    },
    ServerSpec {
        family: "php",
        command: "intelephense",
        args: &["--stdio"],
    },
    ServerSpec {
        family: "perl",
        command: "perlnavigator",
        args: &["--stdio"],
    },
];

/// Which of the known servers are actually installed on this machine.
pub fn available_servers() -> Vec<&'static ServerSpec> {
    SERVERS
        .iter()
        .filter(|spec| on_path(spec.command))
        .collect()
}

fn on_path(command: &str) -> bool {
    if command.contains('/') {
        return is_executable(Path::new(command));
    }
    let Some(search) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&search).any(|directory| is_executable(&directory.join(command)))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    // forgeguard: allow FG-SEC-007 -- a PATH entry joined with a name from the table above, never user input
    fs::metadata(path).is_ok_and(|data| data.is_file() && data.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    // forgeguard: allow FG-SEC-007 -- a PATH entry joined with a name from the table above, never user input
    fs::metadata(path).is_ok_and(|data| data.is_file())
}

#[derive(Debug, Clone)]
pub struct LspOptions {
    /// Per-request timeout. A language server that is indexing must not hang an index run.
    pub timeout: Duration,
    /// Total wall-clock budget across every request for one index run.
    pub budget: Duration,
    /// Families to use; empty means every installed server.
    pub families: Vec<String>,
}

impl Default for LspOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(2),
            budget: Duration::from_secs(30),
            families: Vec::new(),
        }
    }
}

/// Counters an index run can report; none of them change what is stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LspStats {
    pub requests: u64,
    pub hits: u64,
    pub timeouts: u64,
    pub servers_started: u64,
    pub families_skipped: u64,
    /// Requests dropped because the run's wall-clock budget was already spent.
    pub budget_skips: u64,
    /// Answers the client could not use: an error reply or an unparseable shape.
    pub protocol_errors: u64,
}

/// Why a request could not be answered. None of these reach the caller as an
/// error: they disable a family and turn into `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    /// No server for this family, or it could not be spawned or read.
    Missing,
    Timeout,
    /// Malformed frame, error response, or a reply we cannot use.
    Protocol,
}

pub struct LspResolver {
    root: PathBuf,
    options: LspOptions,
    specs: Vec<&'static ServerSpec>,
    servers: HashMap<String, Server>,
    disabled: HashSet<String>,
    /// Protocol errors seen per family. One bad answer is a bad question; a
    /// stream of them is a server we cannot talk to.
    protocol_errors: HashMap<String, u32>,
    deadline: Instant,
    stats: LspStats,
}

/// Protocol errors tolerated per family before it is dropped for this run.
const MAX_PROTOCOL_ERRORS: u32 = 3;

impl LspResolver {
    /// Start nothing yet: servers spawn lazily, on the first request for their family.
    pub fn new(root: &Path, options: LspOptions) -> Self {
        Self::with_servers(root, options, available_servers())
    }

    pub(crate) fn with_servers(
        root: &Path,
        options: LspOptions,
        specs: Vec<&'static ServerSpec>,
    ) -> Self {
        Self {
            root: root.to_path_buf(),
            deadline: Instant::now() + options.budget,
            options,
            specs,
            servers: HashMap::new(),
            disabled: HashSet::new(),
            protocol_errors: HashMap::new(),
            stats: LspStats::default(),
        }
    }

    /// Owning type for the symbol at this position, or `None` when the server is
    /// missing, slow, or ambiguous. `line` is 1-based, `character` 0-based, which
    /// is how the store records a call site. Never an error that fails an index run.
    pub fn receiver_at(
        &mut self,
        path: &Path,
        family: &str,
        line: usize,
        character: usize,
    ) -> Option<String> {
        if Instant::now() >= self.deadline {
            self.stats.budget_skips += 1;
            return None;
        }
        if self.disabled.contains(family) {
            return None;
        }
        if !self.options.families.is_empty() && !self.options.families.iter().any(|f| f == family) {
            self.stats.families_skipped += 1;
            return None;
        }
        self.stats.requests += 1;
        match self.resolve(path, family, line, character) {
            Ok(Some(name)) => {
                self.stats.hits += 1;
                Some(name)
            }
            Ok(None) => None,
            Err(failure) => {
                self.fail(family, failure);
                None
            }
        }
    }

    /// Counters: requests, hits, timeouts, servers started, families skipped.
    pub fn stats(&self) -> LspStats {
        self.stats.clone()
    }

    /// Shut every spawned server down cleanly.
    pub fn shutdown(&mut self) {
        let timeout = self.options.timeout;
        let running: Vec<Server> = self.servers.drain().map(|(_, server)| server).collect();
        for server in running {
            server.stop(timeout);
        }
    }

    fn resolve(
        &mut self,
        path: &Path,
        family: &str,
        line: usize,
        character: usize,
    ) -> Result<Option<String>, Failure> {
        let uri = path_uri(path).ok_or(Failure::Protocol)?;
        self.start(family)?;
        self.open(family, &uri)?;
        // Call sites are stored with 1-based lines, matching every other
        // ForgeGuard report; LSP positions are 0-based.
        let position = json!({
            "textDocument": { "uri": uri },
            "position": { "line": line.saturating_sub(1), "character": character },
        });

        let types = self.ask(family, "textDocument/typeDefinition", &position)?;
        if let Some(name) = name_at_target(&types) {
            return Ok(Some(name));
        }
        let hover = self.ask(family, "textDocument/hover", &position)?;
        if let Some(name) = type_from_hover(&hover) {
            return Ok(Some(name));
        }

        // Last resort: the definition of the callee itself. It only tells us
        // something when that definition is a method inside a container.
        let definition = self.ask(family, "textDocument/definition", &position)?;
        let Some((target, target_line, target_character)) = single_location(&definition) else {
            return Ok(None);
        };
        self.open(family, &target)?;
        let symbols = self.ask(
            family,
            "textDocument/documentSymbol",
            &json!({ "textDocument": { "uri": target } }),
        )?;
        Ok(container_of_method(&symbols, target_line, target_character))
    }

    fn start(&mut self, family: &str) -> Result<(), Failure> {
        if self.servers.contains_key(family) {
            return Ok(());
        }
        let spec = self
            .specs
            .iter()
            .copied()
            .find(|spec| spec.family == family)
            .ok_or(Failure::Missing)?;
        let server = Server::start(spec, &self.root, self.options.timeout)?;
        self.stats.servers_started += 1;
        self.servers.insert(family.to_owned(), server);
        Ok(())
    }

    /// `didOpen` the file behind this URI, once per server. Servers answer
    /// position requests only for documents they have been given.
    fn open(&mut self, family: &str, uri: &str) -> Result<(), Failure> {
        let path = uri_path(uri).ok_or(Failure::Protocol)?;
        let text = read_source(&path).ok_or(Failure::Protocol)?;
        let language = language_id(&path, family);
        let server = self.servers.get_mut(family).ok_or(Failure::Missing)?;
        server.open(uri, &language, &text)
    }

    fn ask(&mut self, family: &str, method: &str, params: &Value) -> Result<Value, Failure> {
        let timeout = self.options.timeout;
        let server = self.servers.get_mut(family).ok_or(Failure::Missing)?;
        server.request(method, params, timeout)
    }

    /// One bad answer disables the family: retrying a hung or broken server for
    /// every remaining call would cost the whole budget.
    fn fail(&mut self, family: &str, failure: Failure) {
        if failure == Failure::Protocol {
            let seen = self.protocol_errors.entry(family.to_owned()).or_default();
            *seen += 1;
            self.stats.protocol_errors += 1;
            if *seen < MAX_PROTOCOL_ERRORS {
                // A single unanswerable position is normal: a macro body, a
                // generated file, a spot the server has no type for.
                return;
            }
        }
        match failure {
            Failure::Timeout => self.stats.timeouts += 1,
            Failure::Missing => self.stats.families_skipped += 1,
            Failure::Protocol => {}
        }
        self.disabled.insert(family.to_owned());
        if let Some(server) = self.servers.remove(family) {
            server.kill();
        }
    }
}

impl Drop for LspResolver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

const POLL: Duration = Duration::from_millis(10);
/// A frame larger than this is a broken server, not a reply we want to allocate for.
const MAX_FRAME: usize = 8 * 1024 * 1024;

struct Server {
    child: Child,
    stdin: ChildStdin,
    incoming: Receiver<Value>,
    next_id: u64,
    opened: HashSet<String>,
}

impl Server {
    fn start(spec: &ServerSpec, root: &Path, timeout: Duration) -> Result<Self, Failure> {
        let mut child = Command::new(spec.command)
            .args(spec.args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| Failure::Missing)?;
        let stdin = child.stdin.take().ok_or(Failure::Protocol)?;
        let stdout = child.stdout.take().ok_or(Failure::Protocol)?;
        // A dedicated reader keeps a silent server from blocking us: the main
        // thread only ever waits on the channel, with a deadline.
        let (sender, incoming) = mpsc::channel();
        thread::spawn(move || read_frames(stdout, &sender));
        let mut server = Self {
            child,
            stdin,
            incoming,
            next_id: 0,
            opened: HashSet::new(),
        };
        match server.handshake(root, timeout) {
            Ok(()) => Ok(server),
            Err(failure) => {
                server.kill();
                Err(failure)
            }
        }
    }

    fn handshake(&mut self, root: &Path, timeout: Duration) -> Result<(), Failure> {
        let root_uri = path_uri(root).ok_or(Failure::Protocol)?;
        let params = json!({
            "processId": Value::Null,
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "synchronization": { "didSave": false },
                    "hover": { "contentFormat": ["markdown", "plaintext"] },
                    "definition": { "linkSupport": true },
                    "typeDefinition": { "linkSupport": true },
                    "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                },
                "workspace": { "workspaceFolders": false },
            },
        });
        self.request("initialize", &params, timeout)?;
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {},
        }))
    }

    fn open(&mut self, uri: &str, language: &str, text: &str) -> Result<(), Failure> {
        if self.opened.contains(uri) {
            return Ok(());
        }
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": {
                "uri": uri,
                "languageId": language,
                "version": 1,
                "text": text,
            }},
        }))?;
        self.opened.insert(uri.to_owned());
        Ok(())
    }

    fn request(
        &mut self,
        method: &str,
        params: &Value,
        timeout: Duration,
    ) -> Result<Value, Failure> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Failure::Timeout);
            }
            let message = match self.incoming.recv_timeout(remaining) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => return Err(Failure::Timeout),
                Err(RecvTimeoutError::Disconnected) => return Err(Failure::Protocol),
            };
            // Progress notifications and server-initiated requests share the
            // stream; we answer none of them and wait for our own id.
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if message.get("error").is_some() {
                return Err(Failure::Protocol);
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn send(&mut self, message: &Value) -> Result<(), Failure> {
        let body = serde_json::to_vec(message).map_err(|_| Failure::Protocol)?;
        write!(self.stdin, "Content-Length: {}\r\n\r\n", body.len())
            .map_err(|_| Failure::Protocol)?;
        self.stdin.write_all(&body).map_err(|_| Failure::Protocol)?;
        self.stdin.flush().map_err(|_| Failure::Protocol)
    }

    fn stop(mut self, timeout: Duration) {
        let _ = self.request("shutdown", &Value::Null, timeout);
        let _ = self.send(&json!({ "jsonrpc": "2.0", "method": "exit" }));
        let Self {
            mut child, stdin, ..
        } = self;
        // Closing stdin is the second exit signal, for servers that ignore the first.
        drop(stdin);
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => thread::sleep(POLL),
                Err(_) => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    fn kill(self) {
        let Self {
            mut child, stdin, ..
        } = self;
        drop(stdin);
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn read_frames(stdout: ChildStdout, sender: &Sender<Value>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let Some(length) = read_header(&mut reader) else {
            return;
        };
        if length > MAX_FRAME {
            return;
        }
        let mut body = vec![0u8; length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&body) else {
            return;
        };
        if sender.send(value).is_err() {
            return;
        }
    }
}

fn read_header(reader: &mut impl BufRead) -> Option<usize> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return length;
        }
        let (name, value) = trimmed.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse().ok();
        }
    }
}

fn read_source(path: &Path) -> Option<String> {
    // forgeguard: allow FG-SEC-007 -- the path is the file being indexed or a target the language server itself named
    fs::read_to_string(path).ok()
}

/// The scanner folds every ECMAScript dialect into one family, but a TypeScript
/// server rejects a `.ts` file announced as JavaScript.
fn language_id(path: &Path, family: &str) -> String {
    let extension = path.extension().and_then(OsStr::to_str).unwrap_or_default();
    let dialect = match extension {
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "jsx" => "javascriptreact",
        _ => "",
    };
    if !dialect.is_empty() {
        return dialect.to_owned();
    }
    match family {
        "objc" => "objective-c",
        "shell" => "shellscript",
        other => other,
    }
    .to_owned()
}

fn path_uri(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    let mut uri = String::from("file://");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(char::from(byte));
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    Some(uri)
}

fn uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok().map(PathBuf::from)
}

/// Exactly one location, or nothing: several candidates mean the server could
/// not decide either, and a guess would be worse than silence.
fn single_location(value: &Value) -> Option<(String, u64, u64)> {
    let found: Vec<&Value> = match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(_) => vec![value],
        _ => Vec::new(),
    };
    if found.len() != 1 {
        return None;
    }
    location_parts(found[0])
}

fn location_parts(item: &Value) -> Option<(String, u64, u64)> {
    let uri = item
        .get("uri")
        .or_else(|| item.get("targetUri"))?
        .as_str()?
        .to_owned();
    let range = item
        .get("targetSelectionRange")
        .or_else(|| item.get("targetRange"))
        .or_else(|| item.get("range"))?;
    let start = range.get("start")?;
    let line = start.get("line")?.as_u64()?;
    let character = start.get("character")?.as_u64()?;
    Some((uri, line, character))
}

/// The name the target range points at, read back from the target file.
fn name_at_target(value: &Value) -> Option<String> {
    let (uri, line, character) = single_location(value)?;
    let text = read_source(&uri_path(&uri)?)?;
    let source = text.lines().nth(usize::try_from(line).ok()?)?;
    declared_name(source, usize::try_from(character).ok()?)
}

const DECLARATION_WORDS: &[&str] = &[
    "struct",
    "class",
    "enum",
    "interface",
    "trait",
    "type",
    "record",
    "object",
    "impl",
    "public",
    "private",
    "protected",
    "static",
    "final",
    "abstract",
    "data",
    "sealed",
    "package",
    "export",
    "declare",
    "const",
    "var",
    "let",
    "function",
    "def",
    "fn",
    "pub",
    "typedef",
    "namespace",
];

fn declared_name(line: &str, character: usize) -> Option<String> {
    let word = identifier_at(line, character);
    match word {
        Some(word) if !DECLARATION_WORDS.contains(&word.as_str()) => Some(word),
        // The range covers the whole declaration: take its first real name.
        _ => line
            .split(|c: char| !is_word(c))
            .find(|part| !part.is_empty() && !DECLARATION_WORDS.contains(part))
            .map(str::to_owned),
    }
}

fn identifier_at(line: &str, character: usize) -> Option<String> {
    let characters: Vec<char> = line.chars().collect();
    if character >= characters.len() || !is_word(characters[character]) {
        return None;
    }
    let mut start = character;
    while start > 0 && is_word(characters[start - 1]) {
        start -= 1;
    }
    let mut end = character;
    while end < characters.len() && is_word(characters[end]) {
        end += 1;
    }
    Some(characters[start..end].iter().collect())
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn type_from_hover(value: &Value) -> Option<String> {
    let text = hover_text(value)?;
    // Hover is markdown prose around a code block; the first code-ish line is
    // the only part with a stable shape across servers.
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("```") && !line.starts_with("---"))?;
    type_name(line)
}

fn hover_text(value: &Value) -> Option<String> {
    match value.get("contents")? {
        Value::String(text) => Some(text.clone()),
        Value::Object(map) => map.get("value").and_then(Value::as_str).map(str::to_owned),
        Value::Array(items) => items.iter().find_map(marked_string),
        _ => None,
    }
}

fn marked_string(item: &Value) -> Option<String> {
    match item {
        Value::String(text) => Some(text.clone()),
        Value::Object(map) => map.get("value").and_then(Value::as_str).map(str::to_owned),
        _ => None,
    }
}

/// Anything that does not reduce to one bare identifier is `None`: a hover line
/// we half-understand is exactly the wrong receiver we must not record.
fn type_name(line: &str) -> Option<String> {
    let stripped = match line.rfind(") ") {
        // Pyright and friends prefix a kind marker: `(variable) value: Repo`.
        Some(index) if line.starts_with('(') => &line[index + 2..],
        _ => line,
    };
    let head = stripped.split_whitespace().next()?;
    let tail = match stripped.rfind(": ") {
        Some(index) => &stripped[index + 2..],
        None if DECLARATION_WORDS.contains(&head) => stripped[head.len()..].trim(),
        None => stripped,
    };
    clean_type(tail)
}

fn clean_type(raw: &str) -> Option<String> {
    let mut value = raw.trim();
    if let Some(index) = value.find('<') {
        value = &value[..index];
    }
    value = value
        .trim_start_matches(['&', '*', '(', '['])
        .trim()
        .trim_end_matches([';', ',', '.', ')', ']', '}', '?', '!'])
        .trim();
    for prefix in ["mut ", "dyn ", "impl ", "const ", "readonly "] {
        value = value.strip_prefix(prefix).unwrap_or(value).trim();
    }
    let last = value.rsplit(['.', ':', '\\', '|']).next()?.trim();
    let named = !last.is_empty()
        && last
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && last.chars().all(is_word);
    named.then(|| last.to_owned())
}

/// LSP `SymbolKind.Method`.
const METHOD_KIND: u64 = 6;

fn container_of_method(value: &Value, line: u64, character: u64) -> Option<String> {
    let items = value.as_array()?;
    flat_container(items, line, character)
        .or_else(|| nested_container(items, None, line, character))
}

/// `SymbolInformation[]`: flat, with the container already named.
fn flat_container(items: &[Value], line: u64, character: u64) -> Option<String> {
    let found = items.iter().find(|item| {
        item.get("kind").and_then(Value::as_u64) == Some(METHOD_KIND)
            && item
                .get("location")
                .and_then(|location| location.get("range"))
                .is_some_and(|range| range_contains(range, line, character))
    })?;
    found
        .get("containerName")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// `DocumentSymbol[]`: a tree, where the container is the parent we descended from.
fn nested_container(
    items: &[Value],
    parent: Option<&str>,
    line: u64,
    character: u64,
) -> Option<String> {
    for item in items {
        let Some(range) = item.get("range") else {
            continue;
        };
        if !range_contains(range, line, character) {
            continue;
        }
        let name = item.get("name").and_then(Value::as_str);
        if item.get("kind").and_then(Value::as_u64) == Some(METHOD_KIND) {
            return parent.map(str::to_owned);
        }
        let children = item.get("children").and_then(Value::as_array);
        return children.and_then(|children| nested_container(children, name, line, character));
    }
    None
}

fn range_contains(range: &Value, line: u64, character: u64) -> bool {
    let bound = |key: &str, field: &str| {
        range
            .get(key)
            .and_then(|point| point.get(field))
            .and_then(Value::as_u64)
    };
    let (Some(start_line), Some(end_line)) = (bound("start", "line"), bound("end", "line")) else {
        return false;
    };
    if line < start_line || line > end_line {
        return false;
    }
    let start_character = bound("start", "character").unwrap_or(0);
    let end_character = bound("end", "character").unwrap_or(u64::MAX);
    (line > start_line || character >= start_character)
        && (line < end_line || character <= end_character)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_servers_never_panics() {
        let _ = available_servers();
    }

    #[test]
    fn parses_conservative_hover_types() {
        assert_eq!(type_name("(variable) value: Repo"), Some("Repo".to_owned()));
        assert_eq!(
            type_name("let repo: crate::db::Repo"),
            Some("Repo".to_owned())
        );
        assert_eq!(type_name("struct Repo"), Some("Repo".to_owned()));
        assert_eq!(type_name("Vec<Repo>"), Some("Vec".to_owned()));
        assert_eq!(type_name("fn find(&self) -> Option<Repo>"), None);
        assert_eq!(type_name("some prose sentence here"), None);
    }

    #[test]
    fn finds_container_in_document_symbols() {
        let nested = json!([{
            "name": "Repo",
            "kind": 5,
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 20, "character": 0 } },
            "children": [{
                "name": "find",
                "kind": 6,
                "range": { "start": { "line": 4, "character": 4 }, "end": { "line": 6, "character": 5 } },
            }],
        }]);
        assert_eq!(container_of_method(&nested, 4, 4), Some("Repo".to_owned()));
        assert_eq!(container_of_method(&nested, 18, 0), None);

        let flat = json!([{
            "name": "find",
            "kind": 6,
            "containerName": "Repo",
            "location": { "uri": "file:///x.rs", "range": {
                "start": { "line": 4, "character": 4 }, "end": { "line": 6, "character": 5 } } },
        }]);
        assert_eq!(container_of_method(&flat, 5, 0), Some("Repo".to_owned()));
    }

    #[test]
    fn ambiguous_results_never_resolve() {
        let two = json!([
            { "uri": "file:///a.rs", "range": { "start": { "line": 0, "character": 0 } } },
            { "uri": "file:///b.rs", "range": { "start": { "line": 0, "character": 0 } } },
        ]);
        assert_eq!(single_location(&two), None);
        assert_eq!(single_location(&json!([])), None);
        assert_eq!(single_location(&Value::Null), None);
    }

    #[test]
    fn missing_binary_counts_and_never_panics() {
        let directory = tempfile::tempdir().expect("tempdir");
        let source = directory.path().join("main.rs");
        std::fs::write(&source, "fn main() {}\n").expect("write");
        let mut resolver = LspResolver::with_servers(
            directory.path(),
            LspOptions::default(),
            vec![leak_spec("rust", "/nonexistent/forgeguard-fake-lsp", &[])],
        );
        assert_eq!(resolver.receiver_at(&source, "rust", 0, 3), None);
        // The second call must not spawn again: the family is already disabled.
        assert_eq!(resolver.receiver_at(&source, "rust", 0, 3), None);
        let stats = resolver.stats();
        assert_eq!(stats.requests, 1);
        assert_eq!(stats.families_skipped, 1);
        assert_eq!(stats.servers_started, 0);
    }

    #[test]
    fn exhausted_budget_skips_requests() {
        let directory = tempfile::tempdir().expect("tempdir");
        let source = directory.path().join("main.rs");
        std::fs::write(&source, "fn main() {}\n").expect("write");
        let options = LspOptions {
            budget: Duration::ZERO,
            ..LspOptions::default()
        };
        let mut resolver = LspResolver::with_servers(
            directory.path(),
            options,
            vec![leak_spec("rust", "/nonexistent/forgeguard-fake-lsp", &[])],
        );
        assert_eq!(resolver.receiver_at(&source, "rust", 0, 3), None);
        let stats = resolver.stats();
        assert_eq!(stats.budget_skips, 1);
        assert_eq!(stats.requests, 0);
    }

    fn leak_spec(family: &'static str, command: &str, args: &[&str]) -> &'static ServerSpec {
        let args: Vec<&'static str> = args
            .iter()
            .map(|arg| &*Box::leak(arg.to_string().into_boxed_str()))
            .collect();
        Box::leak(Box::new(ServerSpec {
            family,
            command: Box::leak(command.to_string().into_boxed_str()),
            args: Box::leak(args.into_boxed_slice()),
        }))
    }

    /// CI has no language server installed, so every round-trip test drives a
    /// shell fake instead. Shell fixtures cannot run on the Windows job.
    #[cfg(unix)]
    mod fake {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        const SCRIPT: &str = r#"#!/bin/sh
mode="$1"
target="$2"
send() {
  printf 'Content-Length: %d\r\n\r\n%s' "${#1}" "$1"
}
while IFS= read -r line; do
  line=$(printf '%s' "$line" | tr -d '\r')
  case "$line" in
    Content-Length:*) len=${line#Content-Length: } ;;
    "")
      body=$(dd bs=1 count="$len" 2>/dev/null)
      id=$(printf '%s' "$body" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
      case "$body" in
        *'"initialize"'*)
          send "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"capabilities\":{}}}" ;;
        *typeDefinition*)
          case "$mode" in
            typedef) send "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":[{\"uri\":\"$target\",\"range\":{\"start\":{\"line\":0,\"character\":7},\"end\":{\"line\":0,\"character\":11}}}]}" ;;
            two) send "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":[{\"uri\":\"$target\",\"range\":{\"start\":{\"line\":0,\"character\":7},\"end\":{\"line\":0,\"character\":11}}},{\"uri\":\"$target\",\"range\":{\"start\":{\"line\":1,\"character\":7},\"end\":{\"line\":1,\"character\":11}}}]}" ;;
            *) send "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":null}" ;;
          esac ;;
        *hover*)
          case "$mode" in
            hover) send "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"contents\":{\"kind\":\"markdown\",\"value\":\"\`\`\`rust\nlet repo: crate::db::Repo\n\`\`\`\"}}}" ;;
            *) send "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":null}" ;;
          esac ;;
        *documentSymbol*)
          send "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":[]}" ;;
        *definition*)
          send "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":null}" ;;
        *shutdown*)
          send "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":null}" ;;
      esac ;;
  esac
done
"#;

        struct Fixture {
            directory: tempfile::TempDir,
        }

        impl Fixture {
            fn new(body: &str) -> Self {
                let directory = tempfile::tempdir().expect("tempdir");
                let script = directory.path().join("fake-lsp");
                std::fs::write(&script, body).expect("write script");
                let mut permissions = std::fs::metadata(&script).expect("stat").permissions();
                permissions.set_mode(0o755);
                std::fs::set_permissions(&script, permissions).expect("chmod");
                std::fs::write(
                    directory.path().join("main.rs"),
                    "fn main() { repo.find(); }\n",
                )
                .expect("write source");
                std::fs::write(
                    directory.path().join("target.rs"),
                    "struct Repo {}\nstruct Other {}\n",
                )
                .expect("write target");
                Self { directory }
            }

            fn source(&self) -> PathBuf {
                self.directory.path().join("main.rs")
            }

            fn resolver(&self, mode: &str) -> LspResolver {
                let target = path_uri(&self.directory.path().join("target.rs")).expect("uri");
                let command = self.directory.path().join("fake-lsp");
                let command = command.to_str().expect("utf8 path");
                LspResolver::with_servers(
                    self.directory.path(),
                    LspOptions {
                        timeout: Duration::from_secs(5),
                        ..LspOptions::default()
                    },
                    vec![leak_spec("rust", command, &[mode, &target])],
                )
            }
        }

        #[test]
        fn resolves_through_type_definition() {
            let fixture = Fixture::new(SCRIPT);
            let mut resolver = fixture.resolver("typedef");
            let found = resolver.receiver_at(&fixture.source(), "rust", 0, 12);
            assert_eq!(found, Some("Repo".to_owned()));
            let stats = resolver.stats();
            assert_eq!(
                (stats.requests, stats.hits, stats.servers_started),
                (1, 1, 1)
            );
            resolver.shutdown();
        }

        #[test]
        fn falls_back_to_hover() {
            let fixture = Fixture::new(SCRIPT);
            let mut resolver = fixture.resolver("hover");
            let found = resolver.receiver_at(&fixture.source(), "rust", 0, 12);
            assert_eq!(found, Some("Repo".to_owned()));
            assert_eq!(resolver.stats().hits, 1);
            resolver.shutdown();
        }

        #[test]
        fn two_locations_resolve_to_nothing() {
            let fixture = Fixture::new(SCRIPT);
            let mut resolver = fixture.resolver("two");
            assert_eq!(resolver.receiver_at(&fixture.source(), "rust", 0, 12), None);
            let stats = resolver.stats();
            assert_eq!((stats.requests, stats.hits, stats.timeouts), (1, 0, 0));
            resolver.shutdown();
        }

        #[test]
        fn silent_server_times_out_and_disables_the_family() {
            let fixture = Fixture::new("#!/bin/sh\nexec cat > /dev/null\n");
            let mut resolver = LspResolver::with_servers(
                fixture.directory.path(),
                LspOptions {
                    timeout: Duration::from_millis(150),
                    ..LspOptions::default()
                },
                vec![leak_spec(
                    "rust",
                    fixture
                        .directory
                        .path()
                        .join("fake-lsp")
                        .to_str()
                        .expect("utf8 path"),
                    &[],
                )],
            );
            assert_eq!(resolver.receiver_at(&fixture.source(), "rust", 0, 12), None);
            assert_eq!(resolver.receiver_at(&fixture.source(), "rust", 0, 12), None);
            let stats = resolver.stats();
            assert_eq!(stats.timeouts, 1);
            // The second call was refused by the disabled family, not retried.
            assert_eq!(stats.requests, 1);
        }
    }
}
