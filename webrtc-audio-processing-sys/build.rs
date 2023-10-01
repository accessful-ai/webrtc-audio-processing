use failure::Error;
use regex::Regex;
use reqwest;
use std::{
    env,
    fs::{create_dir, File},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};

const DEPLOYMENT_TARGET_VAR: &str = "MACOSX_DEPLOYMENT_TARGET";
const GITHUB_REPO_OWNER: &str = "wuurrd";
const GITHUB_REPO_NAME: &str = "webrtc-audio-processing";
const RELEASE_TAG: &str = "v0.1.0";
const ASSET_NAME: &str = "webrtc-Windows.zip";

fn out_dir() -> PathBuf {
    std::env::var("OUT_DIR").expect("OUT_DIR environment var not set.").into()
}

mod webrtc {
    use super::*;
    use failure::bail;

    const BUNDLED_SOURCE_PATH: &str = "./webrtc-audio-processing";

    use zip::read::ZipArchive;

    fn unzip_asset(asset_path: &str, destination_path: &str) -> Result<(), Error> {
        let file = File::open(asset_path)?;
        let mut archive = ZipArchive::new(file)?;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;

            // Construct the output path for the file.
            let outpath = format!("{}\\{}", destination_path, file.name());
            if !file.name().ends_with(".a") && !file.name().ends_with(".pdb") {
                continue;
            }

            if file.is_dir() {
                std::fs::create_dir_all(&outpath)?;
            } else {
                if let Some(p) = outpath.rfind('\\') {
                    if p > 0 {
                        std::fs::create_dir_all(&outpath[0..p])?;
                    }
                }
                println!("Creating file: {}", &outpath);
                let mut outfile = File::create(&outpath)?;
                std::io::copy(&mut file, &mut outfile)?;
            }
        }

        Ok(())
    }

    fn download_asset() -> Result<(), Error> {
        let asset_url = format!(
            "https://github.com/{}/{}/releases/download/{}/{}",
            GITHUB_REPO_OWNER, GITHUB_REPO_NAME, RELEASE_TAG, ASSET_NAME
        );

        let response = reqwest::blocking::get(&asset_url).expect("Failed to download the asset");

        if response.status() != 200 {
            panic!("Failed to download the asset: {}", response.status());
        }

        let mut asset_file = File::create(ASSET_NAME).expect("Failed to create the asset file");
        let mut content = Cursor::new(response.bytes()?);
        std::io::copy(&mut content, &mut asset_file).expect("Failed to save the asset");
        let lib_dir = out_dir().join("webrtc-audio-processing").join("lib");
        let lib_dir_str =
            lib_dir.to_str().ok_or(failure::format_err!("Can not create_lib_dir_str"))?;
        if !lib_dir.is_dir() {
            std::fs::create_dir_all(lib_dir_str)?;
        }
        unzip_asset(ASSET_NAME, lib_dir_str)?;
        Ok(())
    }

    pub(super) fn get_build_paths() -> Result<(PathBuf, PathBuf), Error> {
        let include_path = out_dir().join(BUNDLED_SOURCE_PATH);
        let lib_path = if cfg!(target_os = "windows") {
            out_dir()
                .join("webrtc-audio-processing")
                .join("lib")
                .join("webrtc")
                .join("modules")
                .join("audio_processing")
        } else {
            out_dir().join("webrtc-audio-processing").join("lib")
        };
        Ok((include_path, lib_path))
    }

    fn copy_source_to_out_dir() -> Result<PathBuf, Error> {
        use fs_extra::dir::CopyOptions;

        println!("Bundle path: {}", BUNDLED_SOURCE_PATH);
        if Path::new(BUNDLED_SOURCE_PATH).read_dir()?.next().is_none() {
            eprintln!("The webrtc-audio-processing source directory is empty.");
            eprintln!("See the crate README for installation instructions.");
            eprintln!("Remember to clone the repo recursively if building from source.");
            bail!("Aborting compilation because bundled source directory is empty.");
        }

        let out_dir = out_dir();
        println!("Out dir : {}", out_dir.display());
        let mut options = CopyOptions::new();
        options.overwrite = true;
        let cwd = env::current_dir()?;
        let source = cwd.join(BUNDLED_SOURCE_PATH);
        let path = out_dir.join("webrtc-audio-processing");

        println!(
            "Copy from {} to {}, exists: {}",
            source.display(),
            out_dir.display(),
            path.is_dir(),
        );
        if path.is_dir() {
            // If the source directory exists, we delete it
            std::fs::remove_dir_all(&path)?;
        }
        fs_extra::dir::copy(source, &out_dir, &options)?;
        println!("Copied");

        Ok(out_dir.join(BUNDLED_SOURCE_PATH))
    }

    #[cfg(target_os = "windows")]
    pub(super) fn build() -> Result<(), Error> {
        copy_source_to_out_dir()?;
        download_asset()?;
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    pub(super) fn build() -> Result<(), Error> {
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
        meson_cmd.arg(format!("setup"));
        meson_cmd.arg(format!(".."));
        meson_cmd.arg(format!("--prefix=/"));
        meson_cmd.arg(format!("-Ddefault_library=static"));
        if cfg!(target_os = "windows") {
            meson_cmd.arg(format!("--backend=vs"));
        } else {
            meson_cmd.arg(format!("--backend=ninja"));
        }
        println!("Running command: {:?}", meson_cmd);

        let meson_output = meson_cmd.output()?;
        if !meson_output.status.success() {
            return Err(failure::format_err!(
                "Meson build failed {} {}",
                std::str::from_utf8(meson_output.stderr.as_slice())?,
                std::str::from_utf8(meson_output.stdout.as_slice())?,
            ));
        }

        if !cfg!(target_os = "windows") {
            // Build the project using Ninja (you can use another build system if needed)
            let mut ninja_cmd = std::process::Command::new("ninja");
            ninja_cmd.current_dir(&meson_build_dir);

            println!("Running ninja: {:?} in dir: {}", ninja_cmd, meson_build_dir.display());
            let ninja_output = ninja_cmd.output()?;
            if !ninja_output.status.success() {
                return Err(failure::format_err!(
                    "Ninja build failed: {} {}",
                    std::str::from_utf8(ninja_output.stderr.as_slice())?,
                    std::str::from_utf8(ninja_output.stdout.as_slice())?,
                ));
            }
            // Optionally, you can install the built files into the system
            let mut install_cmd = std::process::Command::new("ninja");
            install_cmd.current_dir(&meson_build_dir);
            install_cmd.env("DESTDIR", build_dir);
            install_cmd.arg("install");
            let install_output = install_cmd.output()?;
            if !install_output.status.success() {
                return Err(failure::format_err!(
                    "Installation failed: {} {}",
                    std::str::from_utf8(install_output.stderr.as_slice())?,
                    std::str::from_utf8(install_output.stdout.as_slice())?,
                ));
            }
        } else {
            println!("MSBUILD starting");
            let mut build_cmd = std::process::Command::new("msbuild");
            build_cmd.current_dir(&meson_build_dir);
            build_cmd.arg(".\\webrtc-audio-processing.sln");
            let build_output = build_cmd.output()?;
            if !build_output.status.success() {
                return Err(failure::format_err!(
                    "msbuild failed: {} {}",
                    std::str::from_utf8(build_output.stderr.as_slice())?,
                    std::str::from_utf8(build_output.stdout.as_slice())?,
                ));
            }
        }
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
    webrtc::build()?;
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

    let mut b = cc_build
        .cpp(true)
        .file("src/wrapper.cpp")
        .include(&webrtc_include)
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-deprecated-declarations")
        .flag_if_supported("-std=c++11");

    #[cfg(target_os = "windows")]
    {
        b.define("WEBRTC_WIN", None);
        b.define("_WIN32", None);
        b.define("__STRICT_ANSI__", None);
        b.define("_WINSOCKAPI_", None);
        b.define("NOMINMAX", None);
        b.define("_USE_MATH_DEFINES", None);
    }

    b.out_dir(&out_dir()).compile("webrtc_audio_processing_wrapper");

    println!("cargo:rustc-link-search=native={}", webrtc_lib.display());
    println!("cargo:rustc-link-lib=static=webrtc_audio_processing_wrapper");

    println!("cargo:rerun-if-env-changed={}", DEPLOYMENT_TARGET_VAR);

    println!("cargo:rustc-link-lib=static=webrtc_audio_processing");

    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=dylib=c++");
    } else if cfg!(target_os = "linux") {
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
