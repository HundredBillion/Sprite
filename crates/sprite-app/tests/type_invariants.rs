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
