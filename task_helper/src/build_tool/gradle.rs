use crate::build_tool::{find_closest_module, which_wrapper, BuildTool};
use crate::command::TaskCommand;
use crate::is_debug;
use std::path::PathBuf;

fn write_init_script(port: Option<&str>) -> std::io::Result<PathBuf> {
    let mut temp_file = std::env::temp_dir();
    let file_name = match port {
        Some(p) => format!("zed_debug_{}.gradle", p),
        None => "zed_run.gradle".to_string(),
    };
    temp_file.push(file_name);

    let mut content = String::new();
    content.push_str("allprojects {\n");
    content.push_str("    tasks.withType(Test) {\n");
    content.push_str("        outputs.upToDateWhen { false }\n");
    if let Some(p) = port {
        content.push_str(&format!(
            r#"        debugOptions {{
            port = {}
        }}
"#,
            p
        ));
    }
    content.push_str("    }\n");
    if let Some(p) = port {
        content.push_str("    tasks.withType(JavaExec) {\n");
        content.push_str("        outputs.upToDateWhen { false }\n");
        content.push_str(&format!(
            r#"        debugOptions {{
            port = {}
        }}
"#,
            p
        ));
        content.push_str("    }\n");
    }
    content.push_str("}\n");

    std::fs::write(&temp_file, content)?;
    Ok(temp_file)
}

pub struct Gradle {
    root: PathBuf,
}

impl Gradle {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn find_module(&self, file: &str) -> Option<PathBuf> {
        find_closest_module(file, &self.root, &["build.gradle", "build.gradle.kts"])
    }
}

impl BuildTool for Gradle {
    fn run_class(
        &self,
        file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
    ) -> TaskCommand {
        let command = which_wrapper(&self.root, "gradle");
        let module = self.find_module(file);
        let full_class = match outer {
            Some(o) => format!("{}${}", o, class),
            None => class.to_string(),
        };
        let full_name = if package.is_empty() {
            full_class
        } else {
            format!("{}.{}", package, full_class)
        };

        let task = if let Some(m) = module {
            format!(":{}:run", m.to_string_lossy().replace("/", ":"))
        } else {
            ":run".to_string()
        };

        let mut args = vec![
            task,
            format!("-PmainClass={}", full_name),
            "--console=plain".to_string(),
            "--no-daemon".to_string(),
        ];

        if is_debug() {
            args.push("--debug-jvm".to_string());
            if let Ok(init_script) = write_init_script(Some(&crate::get_debug_port())) {
                args.push("-I".to_string());
                args.push(init_script.to_string_lossy().to_string());
            }
        }

        TaskCommand {
            command,
            args,
            cwd: self.root.to_string_lossy().to_string(),
        }
    }

    fn run_test_method(
        &self,
        file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
        method: &str,
    ) -> TaskCommand {
        let command = which_wrapper(&self.root, "gradle");
        let module = self.find_module(file);
        let full_class = match outer {
            Some(o) => format!("{}${}", o, class),
            None => class.to_string(),
        };
        let test_filter = if package.is_empty() {
            format!("{}.{}", full_class, method)
        } else {
            format!("{}.{}.{}", package, full_class, method)
        };

        let task = if let Some(m) = module {
            format!(":{}:test", m.to_string_lossy().replace("/", ":"))
        } else {
            ":test".to_string()
        };

        let mut args = vec![
            task,
            "--tests".to_string(),
            test_filter,
            "--console=plain".to_string(),
            "--no-daemon".to_string(),
        ];
        if is_debug() {
            args.push("--debug-jvm".to_string());
            if let Ok(init_script) = write_init_script(Some(&crate::get_debug_port())) {
                args.push("-I".to_string());
                args.push(init_script.to_string_lossy().to_string());
            }
        } else {
            if let Ok(init_script) = write_init_script(None) {
                args.push("-I".to_string());
                args.push(init_script.to_string_lossy().to_string());
            }
        }

        TaskCommand {
            command,
            args,
            cwd: self.root.to_string_lossy().to_string(),
        }
    }

    fn run_test_class(
        &self,
        file: &str,
        package: &str,
        class: &str,
        outer: Option<&str>,
    ) -> TaskCommand {
        let command = which_wrapper(&self.root, "gradle");
        let module = self.find_module(file);
        let full_class = match outer {
            Some(o) => format!("{}${}", o, class),
            None => class.to_string(),
        };
        let test_filter = if package.is_empty() {
            full_class
        } else {
            format!("{}.{}", package, full_class)
        };

        let task = if let Some(m) = module {
            format!(":{}:test", m.to_string_lossy().replace("/", ":"))
        } else {
            ":test".to_string()
        };

        let mut args = vec![
            task,
            "--tests".to_string(),
            test_filter,
            "--console=plain".to_string(),
            "--no-daemon".to_string(),
        ];
        if is_debug() {
            args.push("--debug-jvm".to_string());
            if let Ok(init_script) = write_init_script(Some(&crate::get_debug_port())) {
                args.push("-I".to_string());
                args.push(init_script.to_string_lossy().to_string());
            }
        } else {
            if let Ok(init_script) = write_init_script(None) {
                args.push("-I".to_string());
                args.push(init_script.to_string_lossy().to_string());
            }
        }

        TaskCommand {
            command,
            args,
            cwd: self.root.to_string_lossy().to_string(),
        }
    }

    fn run_all_tests(&self, file: &str) -> TaskCommand {
        let command = which_wrapper(&self.root, "gradle");
        let module = self.find_module(file);

        let task = if let Some(m) = module {
            format!(":{}:test", m.to_string_lossy().replace("/", ":"))
        } else {
            ":test".to_string()
        };

        let mut args = vec![
            task,
            "--console=plain".to_string(),
            "--no-daemon".to_string(),
        ];
        if is_debug() {
            args.push("--debug-jvm".to_string());
            if let Ok(init_script) = write_init_script(Some(&crate::get_debug_port())) {
                args.push("-I".to_string());
                args.push(init_script.to_string_lossy().to_string());
            }
        } else {
            if let Ok(init_script) = write_init_script(None) {
                args.push("-I".to_string());
                args.push(init_script.to_string_lossy().to_string());
            }
        }

        TaskCommand {
            command,
            args,
            cwd: self.root.to_string_lossy().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_gradle_init_script_generation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let gradle = Gradle::new(temp_dir.path().to_path_buf());

        // Test with debug off
        std::env::remove_var("ZED_JAVA_DEBUG");
        let cmd = gradle.run_test_class("SomeFile.java", "com.example", "SomeFileTest", None);

        // Verify --console=plain is present
        assert!(
            cmd.args.contains(&"--console=plain".to_string()),
            "args should contain --console=plain"
        );

        assert!(
            cmd.args.contains(&"--no-daemon".to_string()),
            "args should contain --no-daemon"
        );

        // Find if -I is present and verify file contents
        let init_script_arg_idx = cmd.args.iter().position(|arg| arg == "-I");
        assert!(
            init_script_arg_idx.is_some(),
            "Init script argument -I should be present"
        );
        let init_script_path = &cmd.args[init_script_arg_idx.unwrap() + 1];
        let content =
            fs::read_to_string(init_script_path).expect("Could not read generated init script");

        assert!(
            content.contains("outputs.upToDateWhen { false }"),
            "Should disable up-to-date checks"
        );
        assert!(
            !content.contains("debugOptions"),
            "Should not contain debugOptions when not debugging"
        );

        // Test with debug on
        std::env::set_var("ZED_JAVA_DEBUG", "1");
        std::env::set_var("ZED_JAVA_DEBUG_PORT", "5006");
        let cmd_debug = gradle.run_test_class("SomeFile.java", "com.example", "SomeFileTest", None);

        // Verify --console=plain is present
        assert!(
            cmd_debug.args.contains(&"--console=plain".to_string()),
            "args should contain --console=plain"
        );

        // Verify --debug-jvm is present when debugging
        assert!(
            cmd_debug.args.contains(&"--debug-jvm".to_string()),
            "args should contain --debug-jvm"
        );

        let init_script_arg_idx_debug = cmd_debug.args.iter().position(|arg| arg == "-I");
        assert!(init_script_arg_idx_debug.is_some());
        let init_script_path_debug = &cmd_debug.args[init_script_arg_idx_debug.unwrap() + 1];
        let content_debug = fs::read_to_string(init_script_path_debug).unwrap();

        assert!(content_debug.contains("outputs.upToDateWhen { false }"));
        assert!(
            content_debug.contains("debugOptions"),
            "Should contain debugOptions when debugging"
        );
        assert!(
            content_debug.contains("port = 5006"),
            "Should set correct debug port"
        );

        // Clean up env vars
        std::env::remove_var("ZED_JAVA_DEBUG");
        std::env::remove_var("ZED_JAVA_DEBUG_PORT");
    }
}
