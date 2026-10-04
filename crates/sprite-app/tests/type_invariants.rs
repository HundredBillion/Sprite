use std::path::{Path, PathBuf};
use std::process::Command;

fn dependency(directory: &Path, scratch: &Path, name: &str) -> PathBuf {
    let mut candidates: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| {
            let name_on_disk = entry.file_name();
            let name_on_disk = name_on_disk.to_string_lossy();
            name_on_disk.starts_with(&format!("lib{name}-")) && name_on_disk.ends_with(".rlib")
        })
        .collect();
    candidates
        .sort_by_key(|entry| std::cmp::Reverse(entry.metadata().unwrap().modified().unwrap()));
    let source = scratch.join(format!("dependency_{name}.rs"));
    std::fs::write(&source, format!("extern crate {name};")).unwrap();
    let mut diagnostics = Vec::new();
    for candidate in candidates {
        let output = rustc()
            .args(["--edition=2024", "--crate-type=lib", "--emit=metadata"])
            .arg(&source)
            .arg("--out-dir")
            .arg(scratch)
            .arg("-L")
            .arg(format!("dependency={}", directory.display()))
            .arg("--extern")
            .arg(format!("{name}={}", candidate.path().display()))
            .output()
            .unwrap();
        if output.status.success() {
            return candidate.path();
        }
        diagnostics.push(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    panic!("no compatible compiled dependency {name}: {diagnostics:?}")
}

fn rustc() -> Command {
    Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
}

#[test]
fn private_type_contracts_compile_only_valid_constructions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = std::env::temp_dir().join(format!("sprite-type-proofs-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let source = include_str!("../src/surface/description.rs");
    let declarations = source
        .split_once("/// A description that parsed")
        .expect("Element declaration section boundary")
        .0;
    let (before, rest) = declarations.split_once("impl ColorRef {").unwrap();
    let (_, after) = rest.split_once("/// A grid's size in cells.").unwrap();
    let declarations = format!("{before}{after}")
        .replace("use serde_json::Value;", "")
        .replace("use crate::config::Colors;", "")
        .replace("use crate::surface::Refusal;", "")
        .replace("use crate::tokens::{Role, TokenRegistry};", "");
    let description = scratch.join("description.rs");
    std::fs::write(&description, declarations).unwrap();
    let prelude = format!(
        "#![allow(dead_code, unused_imports, unused_variables)]\n\
         #[path = {:?}] mod grid;\n#[path = {:?}] mod box_drawing;\n\
         mod surface {{ #[path = {:?}] pub mod style; #[path = {:?}] pub mod description; }}\n",
        root.join("src/grid.rs"),
        root.join("src/box_drawing.rs"),
        root.join("src/surface/style.rs"),
        description,
    );
    let dependencies = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let externs: Vec<_> = ["gpui", "sprite_term"]
        .map(|name| {
            format!(
                "{name}={}",
                dependency(&dependencies, &scratch, name).display()
            )
        })
        .into();
    let docs = include_str!("type-invariants/README.md");
    let mut controls = 0;
    let mut refusals = 0;
    for (index, block) in docs
        .split("```")
        .enumerate()
        .filter(|(index, _)| index % 2 == 1)
    {
        let (header, body) = block.split_once('\n').unwrap();
        let file = scratch.join(format!("probe_{index}.rs"));
        std::fs::write(&file, format!("{prelude}\nfn probe() {{\n{body}\n}}\n")).unwrap();
        let mut command = rustc();
        command
            .args(["--edition=2024", "--crate-type=lib", "--emit=metadata"])
            .arg(&file)
            .arg("--out-dir")
            .arg(&scratch)
            .arg("-L")
            .arg(format!("dependency={}", dependencies.display()));
        for external in &externs {
            command.arg("--extern").arg(external);
        }
        let output = command.output().unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        if header == "rust" {
            assert!(output.status.success(), "control {index}: {diagnostic}");
            controls += 1;
        } else {
            let fields: Vec<_> = header.split(',').collect();
            assert_eq!(fields[0], "compile_fail");
            assert!(
                !output.status.success(),
                "negative probe {index} unexpectedly compiled"
            );
            assert!(
                diagnostic.contains(&format!("error[{}]", fields[1])),
                "probe {index}: {diagnostic}"
            );
            assert!(
                diagnostic.contains(fields[2]),
                "probe {index}: {diagnostic}"
            );
            assert!(
                !diagnostic.contains("error[E0603]") && !diagnostic.contains("error[E0432]"),
                "import failure: {diagnostic}"
            );
            refusals += 1;
        }
    }
    assert_eq!((controls, refusals), (2, 5));
    std::fs::remove_dir_all(scratch).unwrap();
}

fn public_examples(source: &str, marker: &str) -> Vec<(String, String)> {
    let docs = source
        .split_once(marker)
        .expect("public contract documentation marker")
        .1
        .split_once("#[derive")
        .expect("public contract declaration boundary")
        .0
        .lines()
        .map(|line| line.strip_prefix("///").unwrap_or(line).trim_start())
        .collect::<Vec<_>>()
        .join("\n");
    docs.split("```")
        .enumerate()
        .filter(|(index, _)| index % 2 == 1)
        .map(|(_, block)| {
            let (header, body) = block.split_once('\n').unwrap();
            (header.to_owned(), body.to_owned())
        })
        .collect()
}

fn compile_public(
    scratch: &Path,
    dependencies: &Path,
    externs: &[String],
    name: &str,
    body: &str,
) -> std::process::Output {
    compile_source(
        scratch,
        dependencies,
        externs,
        name,
        &format!(
            "#![allow(unused_imports, unused_variables, dead_code)]\nfn proof() {{\n{body}\n}}"
        ),
    )
}

fn compile_source(
    scratch: &Path,
    dependencies: &Path,
    externs: &[String],
    name: &str,
    source_text: &str,
) -> std::process::Output {
    let source = scratch.join(format!("{name}.rs"));
    std::fs::write(&source, source_text).unwrap();
    let mut command = rustc();
    command
        .args([
            "--edition=2024",
            "--crate-type=lib",
            "--emit=metadata",
            "--error-format=json",
        ])
        .arg(&source)
        .arg("--out-dir")
        .arg(scratch)
        .arg("-L")
        .arg(format!("dependency={}", dependencies.display()));
    for external in externs {
        command.arg("--extern").arg(external);
    }
    command.output().unwrap()
}

fn intended_public_refusal(output: &std::process::Output, code: &str, subject: &str) -> bool {
    let diagnostics: Vec<serde_json::Value> = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(|line| serde_json::from_str(line).expect("rustc JSON diagnostic"))
        .filter(|diagnostic: &serde_json::Value| diagnostic["level"] == "error")
        .collect();
    let matches = |diagnostic: &serde_json::Value| {
        diagnostic["code"]["code"] == code
            && diagnostic["rendered"]
                .as_str()
                .is_some_and(|message| message.contains(subject))
    };
    !output.status.success()
        && diagnostics.iter().any(&matches)
        && diagnostics.iter().all(|diagnostic| {
            matches(diagnostic)
                || (diagnostic["code"].is_null()
                    && diagnostic["message"]
                        .as_str()
                        .is_some_and(|message| message.starts_with("aborting due to")))
        })
}

#[test]
fn public_doctests_fail_for_the_documented_contract_and_no_other_error() {
    let scratch =
        std::env::temp_dir().join(format!("sprite-public-type-proofs-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let dependencies = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let externs: Vec<_> = ["sprite_app", "sprite_term"]
        .map(|name| {
            format!(
                "{name}={}",
                dependency(&dependencies, &scratch, name).display()
            )
        })
        .into();
    let contracts = [
        (
            "placement",
            public_examples(
                include_str!("../src/surface/channel.rs"),
                "/// Placement constrains ownership to the positions that support it.",
            ),
            vec![("E0559", "return_target"), ("E0063", "return_target")],
        ),
        (
            "terminal_size",
            public_examples(
                include_str!("../../sprite-term/src/lib.rs"),
                "/// Dimensions accepted by both terminal backends.",
            ),
            vec![("E0308", "ValidTerminalSize")],
        ),
    ];
    for (name, examples, expectations) in contracts {
        assert_eq!(examples.len(), expectations.len() + 1);
        assert_eq!(examples[0].0, "");
        let imports = |body: &str| {
            body.lines()
                .filter(|line| line.starts_with("use "))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let control = compile_public(&scratch, &dependencies, &externs, name, &examples[0].1);
        assert!(
            control.status.success(),
            "{name} control: {}",
            String::from_utf8_lossy(&control.stderr)
        );
        for (index, ((header, body), (code, subject))) in
            examples[1..].iter().zip(expectations).enumerate()
        {
            assert_eq!(header, &format!("compile_fail,{code}"));
            assert_eq!(
                imports(body),
                imports(&examples[0].1),
                "{name} control imports"
            );
            let output = compile_public(
                &scratch,
                &dependencies,
                &externs,
                &format!("{name}_{index}"),
                body,
            );
            assert!(
                intended_public_refusal(&output, code, subject),
                "{name} negative {index}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let unrelated = compile_public(
                &scratch,
                &dependencies,
                &externs,
                &format!("{name}_{index}_unrelated"),
                &format!("{}\nlet value = unrelated_symbol;", imports(body)),
            );
            assert!(
                intended_public_refusal(&unrelated, "E0425", "unrelated_symbol"),
                "sensitivity fixture: {}",
                String::from_utf8_lossy(&unrelated.stderr)
            );
            assert!(
                !intended_public_refusal(&unrelated, code, subject),
                "{name} accepted an unrelated error"
            );
            let mixed = compile_public(
                &scratch,
                &dependencies,
                &externs,
                &format!("{name}_{index}_mixed"),
                &format!("{body}\nlet value = unrelated_symbol;"),
            );
            assert!(
                !intended_public_refusal(&mixed, code, subject),
                "{name} accepted an additional unrelated error"
            );
        }
    }
    std::fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn settings_cannot_bypass_validation_or_canonical_collection_construction() {
    let scratch = std::env::temp_dir().join(format!(
        "sprite-settings-type-proofs-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    let dependencies = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let externs: Vec<_> = ["gpui", "sprite_term", "serde", "toml", "image"]
        .iter()
        .map(|name| {
            format!(
                "{name}={}",
                dependency(&dependencies, &scratch, name).display()
            )
        })
        .collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut config_source = include_str!("../src/config.rs").to_owned();
    for module in ["raw", "validated", "changes"] {
        config_source = config_source.replace(
            &format!("mod {module};"),
            &format!(
                "#[path = {:?}] mod {module};",
                root.join(format!("src/config/{module}.rs"))
            ),
        );
    }
    let config_fixture = scratch.join("config.rs");
    std::fs::write(&config_fixture, config_source).unwrap();
    let prelude = format!(
        "#![allow(dead_code, unused_imports, unused_variables)]\n#[path = {:?}] mod config;\n#[path = {:?}] mod graphics_cache;\n#[path = {:?}] mod local_socket;\n",
        config_fixture,
        root.join("src/graphics_cache.rs"),
        root.join("src/local_socket.rs")
    );
    let compile = |name: &str, body: &str| {
        compile_source(
            &scratch,
            &dependencies,
            &externs,
            name,
            &format!("{prelude}\nfn proof() {{\n{body}\n}}"),
        )
    };
    let control = "use config::{Settings, FontSize, LineHeight, Padding};\nlet mut settings = Settings::default();\nsettings.font.size = FontSize::new(f32::NAN);\nsettings.font.line_height = LineHeight::new(0.0);\nsettings.grid.padding = Padding::new(-1.0);";
    let output = compile("settings_valid", control);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for (index, (body, code, subject)) in [
        (
            "let mut settings = config::Settings::default(); settings.font.size = f32::NAN;",
            "E0308",
            "FontSize",
        ),
        (
            "let mut settings = config::Settings::default(); settings.font.line_height = 0.0;",
            "E0308",
            "LineHeight",
        ),
        (
            "let mut settings = config::Settings::default(); settings.grid.padding = -1.0;",
            "E0308",
            "Padding",
        ),
        (
            "let value = config::FontSize(f32::NAN);",
            "E0423",
            "FontSize",
        ),
        (
            "let mut settings = config::Settings::default(); settings.colors.palette = Vec::new();",
            "E0308",
            "CanonicalMap",
        ),
        (
            "let value = local_socket::ReplyConnection { connections: Default::default(), id: 0 };",
            "E0451",
            "private",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let output = compile(&format!("settings_invalid_{index}"), body);
        assert!(
            intended_public_refusal(&output, code, subject),
            "{index}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(scratch).unwrap();
}
