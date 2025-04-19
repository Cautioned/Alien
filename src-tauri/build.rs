use std::fs;
use std::path::{Path, PathBuf};

// Define copy_dir_all here, used for Windows script copying
fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    fs::create_dir_all(&dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.as_ref().join(entry.file_name());

        // Prevent copying over existing files in the destination (safer)
        if dst_path.exists() {
            println!("cargo:warning=Skipping existing file during copy: {:?}", dst_path);
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
    // --- Start: Restore CSS Build & Favicon Copy Logic --- 
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
        "npm.cmd" // Use npm.cmd on Windows
    } else {
        "npm" // Assume npm is in PATH for non-windows
    };

    // Try specific common Mac paths if default fails
    let mut npm_locations = vec![npm_cmd.to_string()]; // Start with default/windows
    if cfg!(target_os = "macos") {
        npm_locations.extend(vec![
            "/usr/local/bin/npm".to_string(),
            "/opt/homebrew/bin/npm".to_string(),
        ]);
    }

    let mut success = false;
    let mut last_error = String::new();

    for npm_path in npm_locations {
        println!("cargo:warning=Trying npm at: {}", npm_path);
        
        // Run npm run build:css directly (no install)
        match std::process::Command::new(&npm_path)
            .current_dir(&templates_dir)
            .arg("run")
            .arg("build:css")
            .output()
        {
            Ok(css_output) => {
                if css_output.status.success() {
                    println!("cargo:warning=CSS build output: {}", String::from_utf8_lossy(&css_output.stdout));
                    success = true;
                    break; // Success, exit loop
                } else {
                    last_error = format!("CSS build failed using '{}': {}\nStdout: {}\nStderr: {}", 
                                       npm_path, 
                                       css_output.status, 
                                       String::from_utf8_lossy(&css_output.stdout),
                                       String::from_utf8_lossy(&css_output.stderr));
                }
            }
            Err(e) => {
                 last_error = format!("Failed to run command '{}': {}", npm_path, e);
                 // If the command itself failed to run (e.g., not found), continue to next path
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
        if let Ok(metadata) = fs::metadata(&css_file) {
            println!("cargo:warning=CSS file size: {} bytes", metadata.len());
        }
    }

    // Copy favicon.ico from icons to templates
    let favicon_src = manifest_path.join("icons").join("icon.ico");
    let favicon_dst = templates_dir.join("favicon.ico");
    // Copy only if destination doesn't exist or source is newer
    let should_copy = !favicon_dst.exists() || 
                      fs::metadata(&favicon_src).ok().zip(fs::metadata(&favicon_dst).ok())
                      .map_or(true, |(src_meta, dst_meta)| {
                          src_meta.modified().ok().zip(dst_meta.modified().ok())
                          .map_or(false, |(src_time, dst_time)| src_time > dst_time)
                      });

    if should_copy {
        if let Err(e) = fs::copy(&favicon_src, &favicon_dst) {
            println!("cargo:warning=Failed to copy favicon: {}", e);
        } else {
            println!("cargo:warning=Favicon copied successfully");
        }
    } else {
         println!("cargo:warning=Skipping favicon copy, destination is up-to-date.");
    }

    // Tell Cargo to re-run only when relevant source files change
    println!("cargo:rerun-if-changed=templates/src");
    println!("cargo:rerun-if-changed=templates/tailwind.config.js");
    // --- End: Restore CSS Build & Favicon Copy Logic --- 

    tauri_build::build();

    // --- Windows Specific Logic (Seems mostly intact from rebase) ---
    if cfg!(target_os = "windows") {
        let windows_manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let windows_manifest_path = PathBuf::from(&windows_manifest_dir);
        
        // --- Scripts copy logic ---
        let scripts_src = windows_manifest_path.join("scripts");
        let scripts_dst = windows_manifest_path.join("scripts"); 
        if scripts_src.exists() {
             if let Err(e) = fs::create_dir_all(&scripts_dst) { 
                  println!("cargo:warning=Failed to create scripts destination directory: {}", e);
             } else {
                  if let Err(e) = copy_dir_all(&scripts_src, &scripts_dst) {
                       println!("cargo:warning=Failed to copy scripts directory: {}", e);
                  } else {
                       println!("cargo:warning=Scripts directory copied successfully");
                  }
             }
        } else {
             println!("cargo:warning=Scripts source directory not found at {:?}", scripts_src);
        }
        // --- End Scripts copy logic ---
    }
    // --- End Windows Specific Logic ---

    println!("cargo:rerun-if-changed=build.rs");
}
