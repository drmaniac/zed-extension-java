mod config;
mod debugger;
mod downloadable;
mod jdk;
mod jdtls;
mod jdtls_server;
mod language_server;
mod lsp;
mod proxy;
mod task;
mod util;

use std::str::FromStr;

use zed_extension_api::{
    self as zed, AttachRequest, BuildTaskDefinition, BuildTaskDefinitionTemplatePayload, CodeLabel,
    DebugAdapterBinary, DebugRequest, DebugScenario, DebugTaskDefinition, Extension,
    LanguageServerId, StartDebuggingRequestArguments, StartDebuggingRequestArgumentsRequest,
    TaskTemplate, Worktree,
    lsp::{Completion, Symbol},
    register_extension,
    serde_json::{self, Value, json},
};

use crate::{
    downloadable::Downloadable, jdtls_server::JdtlsServer, language_server::LanguageServer,
};

const DEBUG_ADAPTER_NAME: &str = "Java";

struct Java {
    jdtls_server: JdtlsServer,
}

impl Extension for Java {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self {
            jdtls_server: JdtlsServer::new(),
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<zed::Command> {
        match language_server_id.as_ref() {
            JdtlsServer::SERVER_ID => self.jdtls_server.command(language_server_id, worktree),
            id => Err(format!("Unknown language server: {id}")),
        }
    }

    fn language_server_initialization_options(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<Option<Value>> {
        match language_server_id.as_ref() {
            JdtlsServer::SERVER_ID => self
                .jdtls_server
                .initialization_options(language_server_id, worktree),
            _ => Ok(None),
        }
    }

    fn language_server_workspace_configuration(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<Option<Value>> {
        match language_server_id.as_ref() {
            JdtlsServer::SERVER_ID => self
                .jdtls_server
                .workspace_configuration(language_server_id, worktree),
            _ => Ok(None),
        }
    }

    fn label_for_completion(
        &self,
        language_server_id: &LanguageServerId,
        completion: Completion,
    ) -> Option<CodeLabel> {
        match language_server_id.as_ref() {
            JdtlsServer::SERVER_ID => self
                .jdtls_server
                .label_for_completion(language_server_id, completion),
            _ => None,
        }
    }

    fn label_for_symbol(
        &self,
        language_server_id: &LanguageServerId,
        symbol: Symbol,
    ) -> Option<CodeLabel> {
        match language_server_id.as_ref() {
            JdtlsServer::SERVER_ID => self
                .jdtls_server
                .label_for_symbol(language_server_id, symbol),
            _ => None,
        }
    }

    fn get_dap_binary(
        &mut self,
        adapter_name: String,
        config: DebugTaskDefinition,
        _user_provided_debug_adapter_path: Option<String>,
        worktree: &Worktree,
    ) -> zed_extension_api::Result<DebugAdapterBinary, String> {
        if !self.jdtls_server.debugger.loaded() {
            return Err("Debugger plugin is not loaded".to_string());
        }

        if adapter_name != DEBUG_ADAPTER_NAME {
            return Err(format!(
                "Cannot create binary for adapter \"{adapter_name}\""
            ));
        }

        let workspace = worktree.root_path();

        // Parse and translate code lens runnable context to Java debug config
        let mut config_value = Value::from_str(config.config.as_str())
            .map_err(|err| format!("Invalid JSON configuration: {err}"))?;

        if let Some(obj) = config_value.as_object_mut() {
            // Translate generic "program" field to Java-specific fields
            if let Some(mut program) = obj.get("program").and_then(Value::as_str).map(String::from)
            {
                let project_name = obj.get("projectName").and_then(Value::as_str);
                eprintln!(
                    "[DAP] get_dap_binary: program={}, project_name={:?}",
                    program, project_name
                );
                let lsp_resolved = resolve_fqcn_via_lsp(&workspace, &program, project_name);
                eprintln!("[DAP] get_dap_binary: lsp_resolved={:?}", lsp_resolved);

                let resolved = lsp_resolved.or_else(|| {
                    find_fqcn_in_workspace(std::path::Path::new(&worktree.root_path()), &program)
                });
                eprintln!(
                    "[DAP] get_dap_binary: final resolved program={:?}",
                    resolved
                );

                if let Some(correct_program) = resolved {
                    program = correct_program;
                    obj.insert("program".to_string(), Value::String(program.clone()));
                }

                obj.entry("mainClass".to_string())
                    .or_insert(Value::String(program));
            }

            // Ensure request is set
            if !obj.contains_key("request") {
                obj.insert("request".to_string(), Value::String("launch".to_string()));
            }
        }

        let config_str = serde_json::to_string(&config_value)
            .map_err(|err| format!("Failed to serialize debug config: {err}"))?;

        Ok(DebugAdapterBinary {
            command: None,
            arguments: vec![],
            cwd: Some(workspace.clone()),
            envs: vec![],
            request_args: StartDebuggingRequestArguments {
                request: self
                    .dap_request_kind(adapter_name, config_value)
                    .map_err(|err| format!("Failed to determine debug request kind: {err}"))?,
                configuration: self
                    .jdtls_server
                    .debugger
                    .inject_config(worktree, config_str, None)
                    .map_err(|err| format!("Failed to inject debug configuration: {err}"))?,
            },
            connection: Some(zed::resolve_tcp_template(
                self.jdtls_server
                    .debugger
                    .start_session(&workspace)
                    .map_err(|err| format!("Failed to start debug session: {err}"))?,
            )?),
        })
    }

    fn dap_request_kind(
        &mut self,
        adapter_name: String,
        config: Value,
    ) -> Result<StartDebuggingRequestArgumentsRequest, String> {
        if adapter_name != DEBUG_ADAPTER_NAME {
            return Err(format!(
                "Cannot create binary for adapter \"{adapter_name}\""
            ));
        }

        match config.get("request") {
            Some(launch) if launch == "launch" => Ok(StartDebuggingRequestArgumentsRequest::Launch),
            Some(attach) if attach == "attach" => Ok(StartDebuggingRequestArgumentsRequest::Attach),
            Some(value) => Err(format!(
                "Unexpected value for `request` key in Java debug adapter configuration: {value:?}"
            )),
            None => {
                Err("Missing required `request` field in Java debug adapter configuration".into())
            }
        }
    }

    fn dap_config_to_scenario(
        &mut self,
        config: zed::DebugConfig,
    ) -> zed::Result<zed::DebugScenario, String> {
        if !self.jdtls_server.debugger.loaded() {
            return Err("Debugger plugin is not loaded".to_string());
        }

        let workspace = self
            .jdtls_server
            .cached_workspace
            .as_deref()
            .ok_or("LSP workspace not initialized yet")?;

        match config.request {
            zed::DebugRequest::Attach(attach) => {
                let debug_config = if let Some(process_id) = attach.process_id {
                    json!({
                        "request": "attach",
                        "processId": process_id,
                        "stopOnEntry": config.stop_on_entry
                    })
                } else {
                    json!({
                        "request": "attach",
                        "hostName": "localhost",
                        "port": 5005,
                    })
                };

                Ok(zed::DebugScenario {
                    adapter: config.adapter,
                    build: None,
                    tcp_connection: Some(
                        self.jdtls_server
                            .debugger
                            .start_session(workspace)
                            .map_err(|err| format!("Failed to start debug session: {err}"))?,
                    ),
                    label: "Attach to Java process".to_string(),
                    config: debug_config.to_string(),
                })
            }

            zed::DebugRequest::Launch(_launch) => {
                Err("Java Extension doesn't support launching".to_string())
            }
        }
    }

    fn dap_locator_create_scenario(
        &mut self,
        _locator_name: String,
        template: TaskTemplate,
        label: String,
        adapter_name: String,
    ) -> Option<DebugScenario> {
        let args = parse_args(&template.command)?;

        let fqcn = get_fqcn(
            args.get(2).cloned().unwrap_or_default().as_str(),
            args.get(3).cloned().unwrap_or_default().as_str(),
            args.get(4).cloned().unwrap_or_default().as_str(),
        );

        let workspace_folder = template.cwd.clone().unwrap_or_default();

        let lsp_resolved = resolve_fqcn_via_lsp(
            &workspace_folder,
            &fqcn,
            find_project_name(&args[1]).as_deref(),
        );
        eprintln!(
            "[DAP] dap_locator_create_scenario: lsp_resolved={:?}",
            lsp_resolved
        );

        let final_fqcn = lsp_resolved.unwrap_or(fqcn);

        let port: u16 = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map(|addr| addr.port())
            .unwrap_or(5005);

        let mut build_template = template.clone();
        build_template.env.push(("ZED_JAVA_DEBUG".to_string(), "1".to_string()));
        build_template.env.push(("ZED_JAVA_DEBUG_PORT".to_string(), port.to_string()));

        let debug_config = json!({
            "request": "attach",
            "hostName": "localhost",
            "port": port,
            "projectName": find_project_name(&args[1]),
        });

        let mut debug_config_obj = debug_config.as_object().unwrap().clone();
        debug_config_obj.insert("testClass".to_string(), Value::String(final_fqcn));
        if args[0] == "run-test-method" {
            debug_config_obj.insert("testMethod".to_string(), Value::String(args[5].clone()));
        }

        let debug_config_value = Value::Object(debug_config_obj);

        Some(DebugScenario {
            adapter: adapter_name,
            build: Some(BuildTaskDefinition::Template(
                BuildTaskDefinitionTemplatePayload {
                    locator_name: Some("java".to_string()),
                    template: build_template,
                },
            )),
            tcp_connection: None,
            label,
            config: debug_config_value.to_string(),
        })
    }

    fn run_dap_locator(
        &mut self,
        _locator_name: String,
        _template: TaskTemplate,
    ) -> zed_extension_api::Result<DebugRequest, String> {
        Ok(DebugRequest::Attach(
            AttachRequest { process_id: None },
        ))
    }
}

fn find_project_name(file_path: &str) -> Option<String> {
    use std::path::Path;

    let path = Path::new(file_path);
    let mut current = if path.is_file() {
        path.parent()
    } else {
        Some(path)
    };

    while let Some(dir) = current {
        if dir.join("build.gradle").exists()
            || dir.join("build.gradle.kts").exists()
            || dir.join("pom.xml").exists()
        {
            return dir
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string());
        }
        current = dir.parent();
    }
    None
}

fn parse_args(command: &str) -> Option<Vec<String>> {
    let subcommand;
    let start_idx;
    if let Some(idx) = command.find(" run-class ") {
        subcommand = "run-class";
        start_idx = idx + " run-class ".len();
    } else if let Some(idx) = command.find(" run-test-method ") {
        subcommand = "run-test-method";
        start_idx = idx + " run-test-method ".len();
    } else if let Some(idx) = command.find(" run-test-class ") {
        subcommand = "run-test-class";
        start_idx = idx + " run-test-class ".len();
    } else {
        return None;
    }

    let mut args = Vec::new();
    args.push(subcommand.to_string());

    let mut current = String::new();
    let mut in_quotes = false;
    for c in command[start_idx..].chars() {
        if c == '"' {
            if in_quotes {
                args.push(current);
                current = String::new();
                in_quotes = false;
            } else {
                in_quotes = true;
            }
        } else if in_quotes {
            current.push(c);
        }
    }
    Some(args)
}

fn get_fqcn(package: &str, class: &str, outer: &str) -> String {
    let mut fqcn = String::new();
    if !package.is_empty() {
        fqcn.push_str(package);
        fqcn.push('.');
    }
    if !outer.is_empty() && outer != "${ZED_CUSTOM_java_outer_class_name:-}" {
        fqcn.push_str(outer);
        fqcn.push('$');
    }
    fqcn.push_str(class);
    fqcn
}

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct LspWorkspaceSymbol {
    name: String,
    container_name: Option<String>,
    location: LspLocation,
}

#[derive(serde::Deserialize, Debug)]
struct LspLocation {
    uri: String,
}

fn resolve_fqcn_via_lsp(
    workspace: &str,
    program: &str,
    project_name: Option<&str>,
) -> Option<String> {
    eprintln!(
        "[DAP] resolve_fqcn_via_lsp: program={}, project_name={:?}",
        program, project_name
    );
    if program.starts_with('/')
        || program.starts_with('\\')
        || program.contains('/')
        || program.contains('\\')
        || program.starts_with("${")
    {
        eprintln!("[DAP] resolve_fqcn_via_lsp: invalid characters in program name");
        return None;
    }

    let parts: Vec<&str> = program.split('$').collect();
    let outer_class_part = parts[0];
    let outer_class_name = outer_class_part
        .split('.')
        .next_back()
        .unwrap_or(outer_class_part);

    if outer_class_name.is_empty()
        || !outer_class_name
            .chars()
            .next()
            .unwrap_or(' ')
            .is_alphabetic()
    {
        eprintln!(
            "[DAP] resolve_fqcn_via_lsp: outer_class_name={:?} does not look like a class",
            outer_class_name
        );
        return None;
    }

    let symbols: Vec<LspWorkspaceSymbol> =
        match lsp::request(workspace, "workspace/symbol", json!({ "query": outer_class_name })) {
            Ok(syms) => syms,
            Err(e) => {
                eprintln!(
                    "[DAP] resolve_fqcn_via_lsp: workspace/symbol request failed: {:?}",
                    e
                );
                return None;
            }
        };

    eprintln!(
        "[DAP] resolve_fqcn_via_lsp: symbols found count={}",
        symbols.len()
    );
    for (idx, symbol) in symbols.iter().enumerate() {
        eprintln!(
            "[DAP] resolve_fqcn_via_lsp: symbol[{}] name={:?}, container={:?}, uri={:?}",
            idx, symbol.name, symbol.container_name, symbol.location.uri
        );
    }

    for symbol in symbols {
        if symbol.name == outer_class_name {
            if let Some(proj) = project_name {
                let project_path_part = proj.replace('.', "/");
                if !symbol.location.uri.contains(&project_path_part) {
                    let last_segment = proj.split('.').next_back().unwrap_or(proj);
                    if !symbol.location.uri.contains(&format!("/{}/", last_segment)) {
                        eprintln!(
                            "[DAP] resolve_fqcn_via_lsp: project mismatch for symbol name={}, skipping",
                            symbol.name
                        );
                        continue;
                    }
                }
            }

            if let Some(container) = symbol.container_name {
                let mut correct_fqcn = if container.is_empty() {
                    outer_class_name.to_string()
                } else {
                    format!("{container}.{outer_class_name}")
                };

                if parts.len() > 1 {
                    for part in parts.iter().skip(1) {
                        correct_fqcn.push('$');
                        correct_fqcn.push_str(part);
                    }
                }
                return Some(correct_fqcn);
            }
        }
    }
    None
}

fn find_fqcn_in_workspace(root: &std::path::Path, target_fqcn: &str) -> Option<String> {
    let parts: Vec<&str> = target_fqcn.split('$').collect();
    let outer_class_fqcn = parts[0];
    let outer_class_parts: Vec<&str> = outer_class_fqcn.split('.').collect();
    let class_name = outer_class_parts.last()?;
    let target_name = format!("{}.java", class_name);

    let expected_package_suffix = if outer_class_parts.len() > 1 {
        Some(outer_class_parts[..outer_class_parts.len() - 1].join("."))
    } else {
        None
    };

    let path = find_file_in_workspace(root, &target_name, expected_package_suffix.as_deref())?;

    if let Ok(content) = std::fs::read_to_string(&path)
        && let Some(package_line) = content
            .lines()
            .map(|line| line.trim())
            .find(|line| line.starts_with("package ") && line.ends_with(';'))
        && let Some(package_name) = package_line
            .strip_prefix("package ")
            .and_then(|p| p.strip_suffix(';'))
            .map(|p| p.trim())
    {
        let mut correct_fqcn = format!("{package_name}.{class_name}");
        if parts.len() > 1 {
            for part in parts.iter().skip(1) {
                correct_fqcn.push('$');
                correct_fqcn.push_str(part);
            }
        }
        Some(correct_fqcn)
    } else {
        None
    }
}

fn find_file_in_workspace(
    root: &std::path::Path,
    target_name: &str,
    expected_package_suffix: Option<&str>,
) -> Option<std::path::PathBuf> {
    let mut dirs_to_visit = vec![root.to_path_buf()];
    let mut best_match = None;

    while let Some(dir) = dirs_to_visit.pop() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if path.is_dir() {
                    let name = path.file_name()?.to_str()?;
                    if name == ".git"
                        || name == "build"
                        || name == "target"
                        || name == "bin"
                        || name == ".gradle"
                        || name == "node_modules"
                        || name == ".idea"
                        || name == ".settings"
                    {
                        continue;
                    }
                    dirs_to_visit.push(path);
                } else if path.is_file()
                    && path.file_name().and_then(|n| n.to_str()) == Some(target_name)
                {
                    let Some(suffix) = expected_package_suffix else {
                        return Some(path);
                    };

                    if let Ok(content) = std::fs::read_to_string(&path)
                        && let Some(package_line) = content
                            .lines()
                            .map(|line| line.trim())
                            .find(|line| line.starts_with("package ") && line.ends_with(';'))
                        && let Some(package_name) = package_line
                            .strip_prefix("package ")
                            .and_then(|p| p.strip_suffix(';'))
                            .map(|p| p.trim())
                        && (package_name == suffix
                            || package_name.ends_with(&format!(".{}", suffix)))
                    {
                        return Some(path);
                    }
                    if best_match.is_none() {
                        best_match = Some(path);
                    }
                }
            }
        }
    }
    best_match
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_find_fqcn_in_workspace() {
        let temp_dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("test_fqcn_tmp");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        // Create a fake package structure:
        // temp_dir/src/main/java/com/example/MyClass.java
        let pkg_dir = temp_dir
            .join("src")
            .join("main")
            .join("java")
            .join("com")
            .join("example");
        fs::create_dir_all(&pkg_dir).unwrap();

        let file_path = pkg_dir.join("MyClass.java");
        fs::write(&file_path, "package com.example;\npublic class MyClass {}").unwrap();

        // 1. Test correcting an incorrect package prefix
        let corrected = find_fqcn_in_workspace(&temp_dir, "example.MyClass");
        assert_eq!(corrected, Some("com.example.MyClass".to_string()));

        // 2. Test with already correct FQCN
        let corrected = find_fqcn_in_workspace(&temp_dir, "com.example.MyClass");
        assert_eq!(corrected, Some("com.example.MyClass".to_string()));

        // 3. Test nested class
        let corrected = find_fqcn_in_workspace(&temp_dir, "example.MyClass$Inner");
        assert_eq!(corrected, Some("com.example.MyClass$Inner".to_string()));

        // 4. Test nonexistent class
        let corrected = find_fqcn_in_workspace(&temp_dir, "NonExistentClass");
        assert_eq!(corrected, None);

        // Clean up
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_parse_args_and_get_fqcn() {
        // Test run-class
        let cmd = r#"EXT="..."; [ -n "$LOCALAPPDATA" ] && EXT="..."; "$EXT/proxy-bin/java-task-helper" run-class "/path/to/File.java" "com.example" "MyClass" """#;
        let args = parse_args(cmd).unwrap();
        assert_eq!(args[0], "run-class");
        assert_eq!(args[1], "/path/to/File.java");
        assert_eq!(args[2], "com.example");
        assert_eq!(args[3], "MyClass");
        assert_eq!(args[4], "");
        assert_eq!(
            get_fqcn(&args[2], &args[3], &args[4]),
            "com.example.MyClass"
        );

        // Test run-test-method
        let cmd = r#"EXT="..."; .../java-task-helper run-test-method "/path/to/File.java" "com.example" "MyClass" "Outer" "myTestMethod""#;
        let args = parse_args(cmd).unwrap();
        assert_eq!(args[0], "run-test-method");
        assert_eq!(args[1], "/path/to/File.java");
        assert_eq!(args[2], "com.example");
        assert_eq!(args[3], "MyClass");
        assert_eq!(args[4], "Outer");
        assert_eq!(args[5], "myTestMethod");
        assert_eq!(
            get_fqcn(&args[2], &args[3], &args[4]),
            "com.example.Outer$MyClass"
        );
    }

    #[test]
    fn test_dap_locator() {
        let mut java = Java {
            jdtls_server: JdtlsServer::new(),
        };

        let template = zed_extension_api::TaskTemplate {
            label: "Run test method".to_string(),
            command: "java-task-helper run-test-method \"/path/to/File.java\" \"com.example\" \"MyClass\" \"Outer\" \"myTestMethod\"".to_string(),
            args: vec![],
            env: vec![],
            cwd: None,
        };

        let scenario = java
            .dap_locator_create_scenario(
                "java".to_string(),
                template,
                "Debug Java Test".to_string(),
                "Java".to_string(),
            )
            .unwrap();

        assert_eq!(scenario.adapter, "Java");
        assert_eq!(scenario.label, "Debug Java Test");
        assert!(scenario.tcp_connection.is_none());

        // Verify config
        let config_val: serde_json::Value = serde_json::from_str(&scenario.config).unwrap();
        assert_eq!(config_val["request"], "attach");
        assert_eq!(config_val["hostName"], "localhost");

        let port = config_val["port"].as_u64().unwrap();
        assert!(port > 0);

        // Verify build task
        if let Some(zed_extension_api::BuildTaskDefinition::Template(payload)) = scenario.build {
            assert_eq!(payload.locator_name, Some("java".to_string()));
            let envs = &payload.template.env;
            assert!(envs.contains(&("ZED_JAVA_DEBUG".to_string(), "1".to_string())));
            assert!(envs.contains(&("ZED_JAVA_DEBUG_PORT".to_string(), port.to_string())));

            // Verify run_dap_locator
            let debug_request = java
                .run_dap_locator("java".to_string(), payload.template.clone())
                .unwrap();

            if let zed_extension_api::DebugRequest::Attach(attach_req) = debug_request {
                assert!(attach_req.process_id.is_none());
            } else {
                panic!("Expected DebugRequest::Attach");
            }
        } else {
            panic!("Expected BuildTaskDefinition::Template");
        }
    }

    #[test]
    fn test_find_project_name() {
        let temp_dir = std::env::temp_dir().join("test_find_project_name_dir");
        let _ = fs::remove_dir_all(&temp_dir);
        let sub_dir = temp_dir.join("my-sub-module");
        fs::create_dir_all(&sub_dir).unwrap();
        fs::write(sub_dir.join("build.gradle"), "plugins { id 'java' }").unwrap();

        let java_file = sub_dir.join("src/test/java/MyTest.java");
        fs::create_dir_all(java_file.parent().unwrap()).unwrap();
        fs::write(&java_file, "class MyTest {}").unwrap();

        let project_name = find_project_name(java_file.to_str().unwrap());
        assert_eq!(project_name, Some("my-sub-module".to_string()));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}

register_extension!(Java);
