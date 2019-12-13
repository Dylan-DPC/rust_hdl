// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this file,
// You can obtain one at http://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2018, Olof Kraigher olof.kraigher@gmail.com

use lsp_types::*;

use self::fnv::FnvHashMap;
use fnv;
use std::collections::hash_map::Entry;

use self::vhdl_parser::{Config, Diagnostic, Project, Severity, Source, SrcPos, Utf8ToLatin1Error};
use crate::rpc_channel::RpcChannel;
use std::io;
use std::path::Path;
use vhdl_parser;

pub struct VHDLServer<T: RpcChannel + Clone> {
    rpc_channel: T,
    // To have well defined unit tests that are not affected by environment
    use_external_config: bool,
    server: Option<InitializedVHDLServer<T>>,
}

impl<T: RpcChannel + Clone> VHDLServer<T> {
    pub fn new(rpc_channel: T) -> VHDLServer<T> {
        Self::new_external_config(rpc_channel, true)
    }

    fn new_external_config(rpc_channel: T, use_external_config: bool) -> VHDLServer<T> {
        VHDLServer {
            rpc_channel,
            use_external_config,
            server: None,
        }
    }

    /// Load configuration file from home folder
    fn load_home_config(&self, config: &mut Config) {
        if let Some(home_dir) = dirs::home_dir() {
            let file_name = home_dir.join(".vhdl_ls.toml");

            if !file_name.exists() {
                return;
            }

            match Config::read_file_path(&file_name) {
                Ok(env_config) => {
                    self.rpc_channel.window_log_message(
                        MessageType::Log,
                        format!(
                            "Loaded HOME folder configuration file: {}",
                            file_name.to_string_lossy()
                        ),
                    );

                    config.append(&env_config);
                }
                Err(ref err) => {
                    self.rpc_channel.window_show_message(
                        MessageType::Error,
                        format!("Error while loading HOME folder variable: {} ", err),
                    );
                }
            }
        }
    }

    /// Load configuration file from environment
    fn load_env_config(&self, config: &mut Config) {
        let env_name = "VHDL_LS_CONFIG";

        if let Some(file_name) = std::env::var_os(env_name) {
            match Config::read_file_path(&Path::new(&file_name)) {
                Ok(env_config) => {
                    self.rpc_channel.window_log_message(
                        MessageType::Log,
                        format!(
                            "Loaded {} configuration file: {}",
                            env_name,
                            file_name.to_string_lossy()
                        ),
                    );

                    config.append(&env_config);
                }
                Err(ref err) => {
                    self.rpc_channel.window_show_message(
                        MessageType::Error,
                        format!(
                            "Error while loading {} environment variable: {} ",
                            env_name, err
                        ),
                    );
                }
            }
        };
    }

    /// Load the vhdl_ls.toml config file from initalizeParams.rootUri
    fn load_root_uri_config(&self, init_params: &InitializeParams) -> io::Result<Config> {
        let root_uri = init_params.root_uri.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "initializeParams.rootUri not set")
        })?;

        let root_path = root_uri.to_file_path().map_err(|_| {
            io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "initializeParams.rootUri {:?} not a valid file path",
                    root_uri
                ),
            )
        })?;

        let config_file = root_path.join("vhdl_ls.toml");
        let config = Config::read_file_path(&config_file)?;

        // Log which file was loaded
        self.rpc_channel.window_log_message(
            MessageType::Log,
            format!(
                "Loaded workspace root configuration file: {}",
                config_file.to_str().unwrap()
            ),
        );

        Ok(config)
    }

    /// Load the configuration or use a default configuration if unsuccessful
    /// Log info/error messages to the client
    fn load_config(&self, init_params: &InitializeParams) -> Config {
        let mut config = Config::default();

        if self.use_external_config {
            self.load_home_config(&mut config);
            self.load_env_config(&mut config);
        }

        match self.load_root_uri_config(&init_params) {
            Ok(root_config) => config.append(&root_config),
            Err(ref err) => {
                self.rpc_channel.window_show_message(
                    MessageType::Warning,
                    format!(
                        "Found no vhdl_ls.toml config file in the workspace root path: {}",
                        err
                    ),
                );
                self.rpc_channel.window_show_message(
                    MessageType::Warning,
                    "Found no library mapping, semantic analysis disabled, will perform syntax checking only",
                );
            }
        };

        config
    }

    pub fn initialize_request(
        &mut self,
        params: InitializeParams,
    ) -> jsonrpc_core::Result<InitializeResult> {
        let config = self.load_config(&params);
        let (server, result) = InitializedVHDLServer::new(self.rpc_channel.clone(), config, params);
        self.server = Some(server);
        Ok(result)
    }

    pub fn shutdown_server(&mut self, _params: ()) -> jsonrpc_core::Result<()> {
        self.server = None;
        Ok(())
    }

    fn mut_server(&mut self) -> &mut InitializedVHDLServer<T> {
        self.server.as_mut().expect("Expected initialized server")
    }

    pub fn exit_notification(&mut self, _params: ()) {
        match self.server {
            Some(_) => ::std::process::exit(1),
            None => ::std::process::exit(0),
        }
    }

    pub fn initialized_notification(&mut self, params: &InitializedParams) {
        self.mut_server().initialized_notification(params);
    }

    pub fn text_document_did_change_notification(&mut self, params: &DidChangeTextDocumentParams) {
        self.mut_server()
            .text_document_did_change_notification(&params)
    }

    pub fn text_document_did_open_notification(&mut self, params: &DidOpenTextDocumentParams) {
        self.mut_server()
            .text_document_did_open_notification(&params)
    }

    // textDocument/declaration
    pub fn text_document_declaration(
        &mut self,
        params: &TextDocumentPositionParams,
    ) -> Option<Location> {
        self.mut_server().text_document_declaration(&params)
    }

    // textDocument/definition
    pub fn text_document_definition(
        &mut self,
        params: &TextDocumentPositionParams,
    ) -> Option<Location> {
        self.mut_server().text_document_definition(&params)
    }

    // textDocument/references
    pub fn text_document_references(&mut self, params: &ReferenceParams) -> Vec<Location> {
        self.mut_server().text_document_references(&params)
    }
}

struct InitializedVHDLServer<T: RpcChannel> {
    rpc_channel: T,
    init_params: InitializeParams,
    project: Project,
    // @TODO manage open/close of these to save memory
    source_map: FnvHashMap<Url, Source>,
    files_with_notifications: FnvHashMap<Url, ()>,
}

/// Allow VHDL Server to act as an RpcChannel
impl<T: RpcChannel> RpcChannel for InitializedVHDLServer<T> {
    fn send_notification(
        &self,
        method: impl Into<String>,
        notification: impl serde::ser::Serialize,
    ) {
        self.rpc_channel.send_notification(method, notification);
    }
}

impl<T: RpcChannel + Clone> InitializedVHDLServer<T> {
    pub fn new(
        rpc_channel: T,
        config: Config,
        init_params: InitializeParams,
    ) -> (InitializedVHDLServer<T>, InitializeResult) {
        // @TODO read num_threads from config file
        let num_threads = 4;
        let mut messages = Vec::new();
        let project = Project::from_config(&config, num_threads, &mut messages);

        for message in messages {
            rpc_channel.window_show_message_struct(&message);
        }

        let server = InitializedVHDLServer {
            rpc_channel,
            init_params,
            project,
            source_map: FnvHashMap::default(),
            files_with_notifications: FnvHashMap::default(),
        };

        let mut capabilities = ServerCapabilities::default();
        capabilities.text_document_sync =
            Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::Full));
        capabilities.declaration_provider = Some(true);
        capabilities.definition_provider = Some(true);
        capabilities.references_provider = Some(true);
        let result = InitializeResult {
            capabilities,
            server_info: None,
        };

        (server, result)
    }

    fn client_supports_related_information(&self) -> bool {
        let try_fun = || {
            self.init_params
                .capabilities
                .text_document
                .as_ref()?
                .publish_diagnostics
                .as_ref()?
                .related_information
        };
        try_fun().unwrap_or(false)
    }

    fn publish_diagnostics(&mut self) {
        let supports_related_information = self.client_supports_related_information();
        let diagnostics = self.project.analyse();
        let diagnostics = {
            if supports_related_information {
                diagnostics
            } else {
                flatten_related(diagnostics)
            }
        };

        let mut files_with_notifications =
            std::mem::replace(&mut self.files_with_notifications, FnvHashMap::default());
        for (file_uri, diagnostics) in diagnostics_by_uri(diagnostics).into_iter() {
            let mut lsp_diagnostics = Vec::new();
            for diagnostic in diagnostics {
                lsp_diagnostics.push(to_lsp_diagnostic(diagnostic));
            }

            let publish_diagnostics = PublishDiagnosticsParams {
                uri: file_uri.clone(),
                diagnostics: lsp_diagnostics,
                version: None,
            };

            self.send_notification("textDocument/publishDiagnostics", publish_diagnostics);

            self.files_with_notifications.insert(file_uri.clone(), ());
        }

        for (file_uri, _) in files_with_notifications.drain() {
            // File has no longer any diagnosics, publish empty notification to clear them
            if !self.files_with_notifications.contains_key(&file_uri) {
                let publish_diagnostics = PublishDiagnosticsParams {
                    uri: file_uri.clone(),
                    diagnostics: vec![],
                    version: None,
                };

                self.send_notification("textDocument/publishDiagnostics", publish_diagnostics);
            }
        }
    }

    fn parse_and_publish_diagnostics(&mut self, uri: &Url, code: &str) {
        // @TODO return error to client
        let file_name = uri_to_file_name(&uri);

        match Source::inline_utf8(file_name.to_string(), code) {
            Ok(source) => {
                // @TODO log error to client
                self.source_map.insert(uri.clone(), source.clone());
                self.project.update_source(&source).unwrap();
                self.publish_diagnostics();
            }
            Err(err) => {
                let publish_diagnostics = PublishDiagnosticsParams {
                    uri: uri.clone(),
                    diagnostics: vec![decode_error_to_lsp_diagnostic(&err)],
                    version: None,
                };

                self.send_notification("textDocument/publishDiagnostics", publish_diagnostics);
            }
        }
    }

    pub fn initialized_notification(&mut self, _params: &InitializedParams) {
        self.publish_diagnostics();
    }

    pub fn text_document_did_change_notification(&mut self, params: &DidChangeTextDocumentParams) {
        self.parse_and_publish_diagnostics(
            &params.text_document.uri,
            &params.content_changes[0].text,
        );
    }

    pub fn text_document_did_open_notification(&mut self, params: &DidOpenTextDocumentParams) {
        self.parse_and_publish_diagnostics(&params.text_document.uri, &params.text_document.text);
    }

    pub fn text_document_declaration(
        &mut self,
        params: &TextDocumentPositionParams,
    ) -> Option<Location> {
        self.source_map
            .get(&params.text_document.uri)
            .and_then(|source| {
                self.project
                    .search_reference(source, position_to_cursor(&params.position))
            })
            .map(|result| srcpos_to_location(&result))
    }

    // Copy goto-declaration for now
    pub fn text_document_definition(
        &mut self,
        params: &TextDocumentPositionParams,
    ) -> Option<Location> {
        self.text_document_declaration(params)
    }

    pub fn text_document_references(&mut self, params: &ReferenceParams) -> Vec<Location> {
        let decl_pos = self
            .source_map
            .get(&params.text_document_position.text_document.uri)
            .and_then(|source| {
                self.project.search_reference(
                    source,
                    position_to_cursor(&params.text_document_position.position),
                )
            });

        if let Some(ref decl_pos) = decl_pos {
            self.project
                .find_all_references(decl_pos)
                .iter()
                .map(|pos| srcpos_to_location(pos))
                .collect()
        } else {
            Vec::new()
        }
    }
}

fn srcpos_to_location(pos: &SrcPos) -> Location {
    let uri = file_name_to_uri(pos.source.file_name());
    Location {
        uri: uri.to_owned(),
        range: to_lsp_range(&pos.range),
    }
}

fn position_to_cursor(position: &lsp_types::Position) -> vhdl_parser::Position {
    vhdl_parser::Position {
        line: position.line,
        character: position.character,
    }
}

fn to_lsp_pos(position: &vhdl_parser::Position) -> lsp_types::Position {
    lsp_types::Position {
        line: position.line,
        character: position.character,
    }
}

fn to_lsp_range(range: &vhdl_parser::Range) -> lsp_types::Range {
    lsp_types::Range {
        start: to_lsp_pos(&range.start),
        end: to_lsp_pos(&range.end),
    }
}

fn diagnostics_by_uri(diagnostics: Vec<Diagnostic>) -> FnvHashMap<Url, Vec<Diagnostic>> {
    let mut map: FnvHashMap<Url, Vec<Diagnostic>> = FnvHashMap::default();

    for diagnostic in diagnostics {
        let uri = file_name_to_uri(diagnostic.pos.source.file_name());
        match map.entry(uri) {
            Entry::Occupied(mut entry) => entry.get_mut().push(diagnostic),
            Entry::Vacant(entry) => {
                let vec = vec![diagnostic];
                entry.insert(vec);
            }
        }
    }

    map
}

fn flatten_related(diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut flat_diagnostics = Vec::new();
    for mut diagnostic in diagnostics {
        flat_diagnostics.extend(diagnostic.drain_related());
        flat_diagnostics.push(diagnostic);
    }
    flat_diagnostics
}

fn file_name_to_uri(file_name: impl AsRef<str>) -> Url {
    // @TODO return error to client
    Url::from_file_path(Path::new(file_name.as_ref())).unwrap()
}

fn uri_to_file_name(uri: &Url) -> String {
    // @TODO return error to client
    uri.to_file_path().unwrap().to_str().unwrap().to_owned()
}

fn decode_error_to_lsp_diagnostic(err: &Utf8ToLatin1Error) -> lsp_types::Diagnostic {
    let range = Range {
        start: lsp_types::Position {
            line: err.line,
            character: err.column,
        },
        end: lsp_types::Position {
            line: err.line,
            character: err.column + 1,
        },
    };

    lsp_types::Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::Error),
        code: None,
        source: Some("vhdl ls".to_owned()),
        message: err.message(),
        related_information: None,
        tags: None,
    }
}

fn to_lsp_diagnostic(diagnostic: Diagnostic) -> lsp_types::Diagnostic {
    let severity = match diagnostic.severity {
        Severity::Error => DiagnosticSeverity::Error,
        Severity::Warning => DiagnosticSeverity::Warning,
        Severity::Info => DiagnosticSeverity::Information,
        Severity::Hint => DiagnosticSeverity::Hint,
    };

    let related_information = if !diagnostic.related.is_empty() {
        let mut related_information = Vec::new();
        for (pos, msg) in diagnostic.related {
            let uri = file_name_to_uri(pos.source.file_name());
            related_information.push(DiagnosticRelatedInformation {
                location: Location {
                    uri: uri.to_owned(),
                    range: to_lsp_range(&pos.range),
                },
                message: msg,
            })
        }
        Some(related_information)
    } else {
        None
    };

    lsp_types::Diagnostic {
        range: to_lsp_range(&diagnostic.pos.range),
        severity: Some(severity),
        code: None,
        source: Some("vhdl ls".to_owned()),
        message: diagnostic.message,
        related_information,
        tags: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc_channel::test_support::*;
    use tempfile;

    fn initialize_server(server: &mut VHDLServer<RpcMock>, root_uri: Url) {
        let capabilities = ClientCapabilities::default();

        #[allow(deprecated)]
        let initialize_params = InitializeParams {
            process_id: None,
            root_path: None,
            root_uri: Some(root_uri),
            initialization_options: None,
            capabilities,
            trace: None,
            workspace_folders: None,
            client_info: None,
        };

        server
            .initialize_request(initialize_params)
            .expect("Should not fail");
        server.initialized_notification(&InitializedParams {});
    }

    fn temp_root_uri() -> (tempfile::TempDir, Url) {
        let tempdir = tempfile::tempdir().unwrap();
        let root_uri = Url::from_file_path(tempdir.path()).unwrap();
        (tempdir, root_uri)
    }

    fn expect_loaded_config_messages(mock: &RpcMock, config_uri: &Url) {
        let file_name = config_uri
            .to_file_path()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        mock.expect_notification_contains(
            "window/logMessage",
            format!("Loaded workspace root configuration file: {}", file_name),
        );
    }

    fn expect_missing_config_messages(mock: &RpcMock) {
        mock.expect_notification_contains(
            "window/showMessage",
            "Found no vhdl_ls.toml config file in the workspace root path",
        );
        mock.expect_notification_contains(
            "window/showMessage",
            "Found no library mapping, semantic analysis disabled, will perform syntax checking only",
        );
    }

    /// Create RpcMock and VHDLServer
    fn setup_server() -> (RpcMock, VHDLServer<RpcMock>) {
        let mock = RpcMock::new();
        let server = VHDLServer::new_external_config(mock.clone(), false);
        (mock, server)
    }

    #[test]
    fn initialize() {
        let (mock, mut server) = setup_server();
        let (_tempdir, root_uri) = temp_root_uri();
        expect_missing_config_messages(&mock);
        initialize_server(&mut server, root_uri);
    }

    #[test]
    fn did_open_no_diagnostics() {
        let (mock, mut server) = setup_server();
        let (_tempdir, root_uri) = temp_root_uri();
        expect_missing_config_messages(&mock);
        initialize_server(&mut server, root_uri.clone());

        let file_url = root_uri.join("ent.vhd").unwrap();
        let code = "
entity ent is
end entity ent;
"
        .to_owned();

        let did_open = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: file_url.clone(),
                language_id: "vhdl".to_owned(),
                version: 0,
                text: code.to_owned(),
            },
        };

        server.text_document_did_open_notification(&did_open);
    }

    #[test]
    fn did_open_with_diagnostics_and_change_without() {
        let (mock, mut server) = setup_server();

        let (_tempdir, root_uri) = temp_root_uri();
        expect_missing_config_messages(&mock);
        initialize_server(&mut server, root_uri.clone());

        let file_url = root_uri.join("ent.vhd").unwrap();
        let code = "
entity ent is
end entity ent2;
"
        .to_owned();

        let did_open = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: file_url.clone(),
                language_id: "vhdl".to_owned(),
                version: 0,
                text: code.to_owned(),
            },
        };

        let publish_diagnostics = PublishDiagnosticsParams {
            uri: file_url.clone(),
            diagnostics: vec![lsp_types::Diagnostic {
                range: Range {
                    start: lsp_types::Position {
                        line: 2,
                        character: "end entity ".len() as u64,
                    },
                    end: lsp_types::Position {
                        line: 2,
                        character: "end entity ent2".len() as u64,
                    },
                },
                code: None,
                severity: Some(DiagnosticSeverity::Error),
                source: Some("vhdl ls".to_owned()),
                message: "End identifier mismatch, expected ent".to_owned(),
                related_information: None,
                tags: None,
            }],
            version: None,
        };

        mock.expect_notification("textDocument/publishDiagnostics", publish_diagnostics);
        server.text_document_did_open_notification(&did_open);

        let code = "
entity ent is
end entity ent;
"
        .to_owned();

        let did_change = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: file_url.clone(),
                version: Some(1),
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: code.clone(),
            }],
        };

        let publish_diagnostics = PublishDiagnosticsParams {
            uri: file_url.clone(),
            diagnostics: vec![],
            version: None,
        };

        mock.expect_notification("textDocument/publishDiagnostics", publish_diagnostics);
        server.text_document_did_change_notification(&did_change);
    }

    fn write_file(root_uri: &Url, file_name: impl AsRef<str>, contents: impl AsRef<str>) -> Url {
        let path = root_uri.to_file_path().unwrap().join(file_name.as_ref());
        std::fs::write(&path, contents.as_ref()).unwrap();
        Url::from_file_path(path).unwrap()
    }

    fn write_config(root_uri: &Url, contents: impl AsRef<str>) -> Url {
        write_file(root_uri, "vhdl_ls.toml", contents)
    }

    #[test]
    fn initialize_with_config() {
        let (mock, mut server) = setup_server();
        let (_tempdir, root_uri) = temp_root_uri();
        let file_uri = write_file(
            &root_uri,
            "file.vhd",
            "\
entity ent is
end entity;

architecture rtl of ent2 is
begin
end;
",
        );

        let config_uri = write_config(
            &root_uri,
            "
[libraries]
lib.files = [
  'file.vhd'
]
",
        );

        let publish_diagnostics = PublishDiagnosticsParams {
            uri: file_uri.clone(),
            diagnostics: vec![lsp_types::Diagnostic {
                range: Range {
                    start: lsp_types::Position {
                        line: 3,
                        character: "architecture rtl of ".len() as u64,
                    },
                    end: lsp_types::Position {
                        line: 3,
                        character: "architecture rtl of ent2".len() as u64,
                    },
                },
                code: None,
                severity: Some(DiagnosticSeverity::Error),
                source: Some("vhdl ls".to_owned()),
                message: "No entity \'ent2\' within library \'lib\'".to_owned(),
                related_information: None,
                tags: None,
            }],
            version: None,
        };

        expect_loaded_config_messages(&mock, &config_uri);
        mock.expect_notification("textDocument/publishDiagnostics", publish_diagnostics);

        initialize_server(&mut server, root_uri);
    }

    #[test]
    fn initialize_with_bad_config() {
        let (mock, mut server) = setup_server();
        let (_tempdir, root_uri) = temp_root_uri();

        write_config(
            &root_uri,
            "
[libraries
",
        );
        mock.expect_notification_contains(
            "window/showMessage",
            "Found no vhdl_ls.toml config file in the workspace root path",
        );
        mock.expect_notification_contains(
            "window/showMessage",
            "Found no library mapping, semantic analysis disabled, will perform syntax checking only",
        );
        initialize_server(&mut server, root_uri);
    }

    #[test]
    fn initialize_with_config_missing_files() {
        let (mock, mut server) = setup_server();
        let (_tempdir, root_uri) = temp_root_uri();

        let config_uri = write_config(
            &root_uri,
            "
[libraries]
lib.files = [
'missing_file.vhd',
]
",
        );

        expect_loaded_config_messages(&mock, &config_uri);
        mock.expect_notification_contains("window/showMessage", "missing_file.vhd");
        initialize_server(&mut server, root_uri);
    }

    #[test]
    fn utf8_to_latin1_conversion_gives_error_diagnostic() {
        let (mock, mut server) = setup_server();
        let (_tempdir, root_uri) = temp_root_uri();
        expect_missing_config_messages(&mock);
        initialize_server(&mut server, root_uri.clone());

        let file_url = root_uri.join("ent.vhd").unwrap();
        let code = "\
entity ent is
-- €
end entity ent;
"
        .to_owned();

        let did_open = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: file_url.clone(),
                language_id: "vhdl".to_owned(),
                version: 0,
                text: code.to_owned(),
            },
        };

        let publish_diagnostics = PublishDiagnosticsParams {
            uri: file_url.clone(),
            diagnostics: vec![lsp_types::Diagnostic {
                range: Range {
                    start: lsp_types::Position {
                        line: 1,
                        character: 3,
                    },
                    end: lsp_types::Position {
                        line: 1,
                        character: 4,
                    },
                },
                code: None,
                severity: Some(DiagnosticSeverity::Error),
                source: Some("vhdl ls".to_owned()),
                message: "Found invalid latin-1 character '€' when decoding from utf-8".to_owned(),
                related_information: None,
                tags: None,
            }],
            version: None,
        };

        mock.expect_notification("textDocument/publishDiagnostics", publish_diagnostics);
        server.text_document_did_open_notification(&did_open);
    }

    #[test]
    fn text_document_declaration() {
        let (mock, mut server) = setup_server();
        let (_tempdir, root_uri) = temp_root_uri();

        let file_url1 = write_file(
            &root_uri,
            "pkg1.vhd",
            "\
package pkg1 is
  type typ_t is (foo, bar);
end package;
",
        );

        let code2 = "\
use work.pkg1.all;
package pkg2 is
  constant c : typ_t := bar;
end package;
        "
        .to_owned();
        let file_url2 = write_file(&root_uri, "pkg2.vhd", &code2);

        let config_uri = write_config(
            &root_uri,
            "
[libraries]
lib.files = [
  '*.vhd'
]
",
        );

        expect_loaded_config_messages(&mock, &config_uri);
        initialize_server(&mut server, root_uri.clone());

        let did_open = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: file_url2.clone(),
                language_id: "vhdl".to_owned(),
                version: 0,
                text: code2.to_owned(),
            },
        };

        server.text_document_did_open_notification(&did_open);

        let response = server.text_document_declaration(&TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: file_url2.clone(),
            },
            position: lsp_types::Position {
                line: 2,
                character: "  constant c : t".len() as u64,
            },
        });

        let expected = Location {
            uri: file_url1.clone(),
            range: Range {
                start: lsp_types::Position {
                    line: 1,
                    character: "  type ".len() as u64,
                },
                end: lsp_types::Position {
                    line: 1,
                    character: "  type tpe_t".len() as u64,
                },
            },
        };

        assert_eq!(response, Some(expected));
    }
}
