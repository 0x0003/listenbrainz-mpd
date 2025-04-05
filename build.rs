#[cfg(not(feature = "shell_completion"))]
fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_SHELL_COMPLETION");
}

#[cfg(feature = "shell_completion")]
mod cli {
    #![allow(dead_code)]
    include!("src/cli.rs");
}

#[cfg(feature = "shell_completion")]
fn main() {
    use std::{env, fs::create_dir_all, path::PathBuf};

    use clap::{CommandFactory, ValueEnum};
    use clap_complete::{Shell, generate_to};

    let out_dir = env::var_os("COMPLETIONS_OUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./generated_completions"));
    create_dir_all(&out_dir).expect("failed to create COMPLETIONS_OUT_DIR");

    let mut cmd = cli::CliArgs::command();
    for shell in Shell::value_variants() {
        generate_to(*shell, &mut cmd, env!("CARGO_PKG_NAME"), &out_dir)
            .expect("failed to generate completions");
    }

    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_SHELL_COMPLETION");
    println!("cargo:rerun-if-env-changed=COMPLETIONS_OUT_DIR");
    println!("cargo:rerun-if-changed=src/cli.rs");
}
