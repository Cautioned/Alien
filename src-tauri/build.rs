use std::fs;
use std::path::Path;

fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    fs::create_dir_all(&dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.as_ref().join(entry.file_name());
        
        // Skip if destination already exists
        if dst_path.exists() {
            println!("cargo:warning=Skipping existing file: {:?}", dst_path);
            continue;
        }

        if ty.is_dir() {
            copy_dir_all(entry.path(), dst_path)?;
        } else {
            println!("cargo:warning=Copying file: {:?} -> {:?}", entry.path(), dst_path);
            fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}

fn main() {
    // Get the manifest directory first since we'll need it
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let manifest_path = Path::new(&manifest_dir);
    let templates_dir = manifest_path.join("templates");
    
    // Ensure the dist directory exists
    let dist_dir = templates_dir.join("dist");
    if let Err(e) = fs::create_dir_all(&dist_dir) {
        panic!("Failed to create dist directory: {}", e);
    }

    // Build the CSS
    println!("cargo:warning=Building CSS...");
    
    // Determine npm command based on platform
    let npm_cmd = if cfg!(target_os = "windows") {
        "npm"
    } else {
        "/usr/local/bin/npm"  // Common location on Mac
    };

    // Try multiple npm locations if the first one fails
    let npm_locations = vec![
        npm_cmd,
        "/opt/homebrew/bin/npm",  // Homebrew npm location
        "/usr/bin/npm",           // Another possible location
        "npm"                     // Fallback to PATH
    ];

    let mut success = false;
    let mut last_error = String::new();

    for npm_path in npm_locations {
        println!("cargo:warning=Trying npm at: {}", npm_path);
        
        // Run npm run build:css
        match std::process::Command::new(npm_path)
            .current_dir(&templates_dir)
            .arg("run")
            .arg("build:css")
            .output()
        {
            Ok(css_output) => {
                if css_output.status.success() {
                    println!("cargo:warning=CSS build output: {}", String::from_utf8_lossy(&css_output.stdout));
                    success = true;
                    break;
                } else {
                    last_error = format!("CSS build failed: {}", String::from_utf8_lossy(&css_output.stderr));
                }
            }
            Err(e) => {
                last_error = format!("Failed to run build:css: {}", e);
            }
        }
    }

    if !success {
        panic!("Failed to build CSS after trying all npm locations. Last error: {}", last_error);
    }

    // Verify the CSS file was created
    let css_file = dist_dir.join("styles.css");
    if !css_file.exists() {
        panic!("CSS file was not created at {:?}", css_file);
    } else {
        println!("cargo:warning=CSS built successfully at {:?}", css_file);
        // Print the file size to verify it's not empty
        if let Ok(metadata) = fs::metadata(&css_file) {
            println!("cargo:warning=CSS file size: {} bytes", metadata.len());
            // Print the first few bytes of the file to verify content
            if let Ok(content) = fs::read_to_string(&css_file) {
                let preview = content.chars().take(100).collect::<String>();
                println!("cargo:warning=CSS file preview: {}", preview);
            }
        }
    }

    // Copy favicon.ico from icons to templates
    let favicon_src = manifest_path.join("icons").join("icon.ico");
    let favicon_dst = templates_dir.join("favicon.ico");
    if let Err(e) = fs::copy(&favicon_src, &favicon_dst) {
        println!("cargo:warning=Failed to copy favicon: {}", e);
    } else {
        println!("cargo:warning=Favicon copied successfully");
    }

    // Tell Cargo to re-run only when relevant source files change (avoid dist output loop)
    println!("cargo:rerun-if-changed=templates/src");
    println!("cargo:rerun-if-changed=templates/tailwind.config.js");
    
    tauri_build::build();

    if cfg!(target_os = "windows") {
        use std::env;
        use std::path::PathBuf;
        use std::process::Command;
        
        // Verify we're building for 64-bit
        if env::var("CARGO_CFG_TARGET_POINTER_WIDTH").unwrap() != "64" {
            panic!("This application requires 64-bit Windows");
        }

        // Get the manifest directory (where Cargo.toml is)
        let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        let manifest_path = PathBuf::from(&manifest_dir);

        // Copy DLL to root app directory
        let dll_path = manifest_path.join("lib").join("64").join("libmpv-2.dll");
        let dst_dll = manifest_path.join("libmpv-2.dll");
        let lib_path = manifest_path.join("mpv.lib");

        // Create scripts directory if it doesn't exist
        let scripts_src = manifest_path.join("scripts");
        if !scripts_src.exists() {
            if let Err(e) = fs::create_dir_all(&scripts_src) {
                println!("cargo:warning=Failed to create scripts directory: {}", e);
            } else {
                println!("cargo:warning=Created scripts directory at {:?}", scripts_src);
            }
        }

        // Copy scripts directory
        let scripts_dst = manifest_path.join("scripts");
        if scripts_src.exists() {
            if let Err(e) = fs::create_dir_all(&scripts_dst) {
                println!("cargo:warning=Failed to create scripts directory: {}", e);
            } else {
                if let Err(e) = copy_dir_all(&scripts_src, &scripts_dst) {
                    println!("cargo:warning=Failed to copy scripts directory: {}", e);
                } else {
                    println!("cargo:warning=Scripts directory copied successfully");
                    // Print the contents of the scripts directory
                    if let Ok(entries) = fs::read_dir(&scripts_dst) {
                        println!("cargo:warning=Contents of scripts directory:");
                        for entry in entries {
                            if let Ok(entry) = entry {
                                println!("cargo:warning=  {:?}", entry.path());
                            }
                        }
                    }
                }
            }
        } else {
            println!("cargo:warning=Scripts directory not found at {:?}", scripts_src);
        }

        // Only generate lib file if it doesn't exist
        if !lib_path.exists() {
            if dll_path.exists() {
                if let Err(e) = fs::copy(&dll_path, &dst_dll) {
                    println!("cargo:warning=Failed to copy libmpv-2.dll: {}", e);
                    println!("cargo:warning=Source path: {:?}", dll_path);
                    println!("cargo:warning=Destination path: {:?}", dst_dll);
                } else {
                    println!("cargo:warning=libmpv-2.dll copied successfully");
                }

                // Run generate-lib.ps1 script
                println!("cargo:warning=Generating lib file...");
                let script_path = manifest_path.join("generate-lib.ps1");
                let output = Command::new("powershell")
                    .args(["-ExecutionPolicy", "Bypass", "-File", script_path.to_str().unwrap()])
                    .current_dir(&manifest_path)
                    .output();

                match output {
                    Ok(output) => {
                        if output.status.success() {
                            println!("cargo:warning=Successfully generated lib file");
                            println!("cargo:warning=stdout: {}", String::from_utf8_lossy(&output.stdout));
                        } else {
                            println!("cargo:warning=Failed to generate lib file");
                            println!("cargo:warning=stderr: {}", String::from_utf8_lossy(&output.stderr));
                        }
                    }
                    Err(e) => {
                        println!("cargo:warning=Failed to run generate-lib.ps1: {}", e);
                    }
                }
            } else {
                println!("cargo:warning=libmpv-2.dll not found at {:?}", dll_path);
            }
        } else {
            println!("cargo:warning=mpv.lib already exists, skipping generation");
        }
    }

    println!("cargo:rerun-if-changed=build.rs");
}
