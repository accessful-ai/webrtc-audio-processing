use failure::Error;
use regex::Regex;
use std::{
    env,
    fs::{create_dir, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const DEPLOYMENT_TARGET_VAR: &str = "MACOSX_DEPLOYMENT_TARGET";

fn out_dir() -> PathBuf {
    std::env::var("OUT_DIR").expect("OUT_DIR environment var not set.").into()
}

mod webrtc {
    use super::*;
    use failure::bail;

    const BUNDLED_SOURCE_PATH: &str = "./webrtc-audio-processing";

    pub(super) fn get_build_paths() -> Result<(PathBuf, PathBuf), Error> {
        let include_path = out_dir().join(BUNDLED_SOURCE_PATH);
        let lib_path = out_dir().join("lib");
        Ok((include_path, lib_path))
    }

    fn copy_source_to_out_dir() -> Result<PathBuf, Error> {
        use fs_extra::dir::CopyOptions;

        if Path::new(BUNDLED_SOURCE_PATH).read_dir()?.next().is_none() {
            eprintln!("The webrtc-audio-processing source directory is empty.");
            eprintln!("See the crate README for installation instructions.");
            eprintln!("Remember to clone the repo recursively if building from source.");
            bail!("Aborting compilation because bundled source directory is empty.");
        }

        let out_dir = out_dir();
        let mut options = CopyOptions::new();
        options.overwrite = true;

        println!("Copy from {} to {}", BUNDLED_SOURCE_PATH, out_dir.display());
        fs_extra::dir::copy(BUNDLED_SOURCE_PATH, &out_dir, &options)?;

        Ok(out_dir.join(BUNDLED_SOURCE_PATH))
    }

    pub(super) fn build_if_necessary() -> Result<(), Error> {
        // Copy source to the output directory (build_dir)
        let build_dir = copy_source_to_out_dir()?;

        // Run Meson for building the dependency
        let meson_build_dir = build_dir.join("build");
        if !meson_build_dir.is_dir() {
            create_dir(&meson_build_dir)?;
        }
        let mut meson_cmd = std::process::Command::new("meson");
        //meson_cmd.current_dir(&build_dir);
        meson_cmd.current_dir(&meson_build_dir);
        meson_cmd.arg(format!("--prefix=/"));

        let meson_output = meson_cmd.output()?;
        if !meson_output.status.success() {
            return Err(failure::format_err!("Meson build failed"));
        }

        // Build the project using Ninja (you can use another build system if needed)
        let mut ninja_cmd = std::process::Command::new("ninja");
        ninja_cmd.current_dir(&meson_build_dir);

        let ninja_output = ninja_cmd.output()?;
        if !ninja_output.status.success() {
            return Err(failure::format_err!("Ninja build failed"));
        }
        // Optionally, you can install the built files into the system
        println!("BUILD IF NECESSARY2");
        let mut install_cmd = std::process::Command::new("ninja");
        install_cmd.current_dir(&meson_build_dir);
        install_cmd.env("DESTDIR", build_dir);
        install_cmd.arg("install");
        let install_output = install_cmd.output()?;
        if !install_output.status.success() {
            return Err(failure::format_err!(
                "Installation failed: {}",
                std::str::from_utf8(install_output.stderr.as_slice())?,
            ));
        }
        Ok(())
    }
    // pub(super) fn build_if_necessary() -> Result<(), Error> {
    //     let build_dir = copy_source_to_out_dir()?;

    //     if cfg!(target_os = "macos") {
    //         run_command(&build_dir, "glibtoolize", None)?;
    //     } else {
    //         run_command(&build_dir, "libtoolize", None)?;
    //     }

    //     run_command(&build_dir, "aclocal", None)?;
    //     run_command(&build_dir, "automake", Some(&["--add-missing", "--copy"]))?;
    //     run_command(&build_dir, "autoconf", None)?;

    //     autotools::Config::new(build_dir)
    //         .cflag("-fPIC")
    //         .cxxflag("-fPIC")
    //         .disable_shared()
    //         .enable_static()
    //         .build();

    //     Ok(())
    // }

    fn run_command<P: AsRef<Path>>(
        curr_dir: P,
        cmd: &str,
        args_opt: Option<&[&str]>,
    ) -> Result<(), Error> {
        let mut command = std::process::Command::new(cmd);

        command.current_dir(curr_dir);

        if let Some(args) = args_opt {
            command.args(args);
        }

        let _output = command.output().map_err(|e| {
            failure::format_err!(
                "Error running command '{}' with args '{:?}' - {:?}",
                cmd,
                args_opt,
                e
            )
        })?;

        Ok(())
    }
}

// TODO: Consider fixing this with the upstream.
// https://github.com/rust-lang/rust-bindgen/issues/1089
// https://github.com/rust-lang/rust-bindgen/issues/1301
fn derive_serde(binding_file: &Path) -> Result<(), Error> {
    let mut contents = String::new();
    File::open(binding_file)?.read_to_string(&mut contents)?;

    let new_contents = format!(
        "use serde::{{Serialize, Deserialize}};\n{}",
        Regex::new(r"#\s*\[\s*derive\s*\((?P<d>[^)]+)\)\s*\]\s*pub\s*(?P<s>struct|enum)")?
            .replace_all(&contents, "#[derive($d, Serialize, Deserialize)] pub $s")
    );

    File::create(&binding_file)?.write_all(new_contents.as_bytes())?;

    Ok(())
}

fn main() -> Result<(), Error> {
    webrtc::build_if_necessary()?;
    let (webrtc_include, webrtc_lib) = webrtc::get_build_paths()?;

    let mut cc_build = cc::Build::new();

    // set mac minimum version
    if cfg!(target_os = "macos") {
        let min_version = match env::var(DEPLOYMENT_TARGET_VAR) {
            Ok(ver) => ver,
            Err(_) => {
                String::from(match std::env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
                    "x86_64" => "10.10", // Using what I found here https://github.com/webrtc-uwp/chromium-build/blob/master/config/mac/mac_sdk.gni#L17
                    "aarch64" => "11.0", // Apple silicon started here.
                    arch => panic!("unknown arch: {}", arch),
                })
            },
        };

        // `cc` doesn't try to pick up on this automatically, but `clang` needs it to
        // generate a "correct" Objective-C symbol table which better matches XCode.
        // See https://github.com/h4llow3En/mac-notification-sys/issues/45.
        cc_build.flag(&format!("-mmacos-version-min={}", min_version));
    }

    cc_build
        .cpp(true)
        .file("src/wrapper.cpp")
        .include(&webrtc_include)
        .flag("-Wno-unused-parameter")
        .flag("-Wno-deprecated-declarations")
        .flag("-std=c++11")
        .out_dir(&out_dir())
        .compile("webrtc_audio_processing_wrapper");

    println!("cargo:rustc-link-search=native={}", webrtc_lib.display());
    println!("cargo:rustc-link-lib=static=webrtc_audio_processing_wrapper");

    println!("cargo:rerun-if-env-changed={}", DEPLOYMENT_TARGET_VAR);

    println!("cargo:rustc-link-lib=static=webrtc_audio_processing");

    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=dylib=c++");
    } else {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }

    let binding_file = out_dir().join("bindings.rs");
    bindgen::Builder::default()
        .header("src/wrapper.hpp")
        .generate_comments(true)
        .rustified_enum(".*")
        .derive_debug(true)
        .derive_default(true)
        .derive_partialeq(true)
        .clang_arg(&format!("-I{}", &webrtc_include.display()))
        .disable_name_namespacing()
        .generate()
        .expect("Unable to generate bindings")
        .write_to_file(&binding_file)
        .expect("Couldn't write bindings!");

    if cfg!(feature = "derive_serde") {
        derive_serde(&binding_file).expect("Failed to modify derive macros");
    }

    Ok(())
}
