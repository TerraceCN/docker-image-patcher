use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result};
use clap::Parser;
use log::{debug, error, info, warn};
use serde::{Deserialize};
use tar::{Archive, Builder};

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    #[serde(rename = "Layers")]
    layers: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct InspectEntry {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "RootFS")]
    rootfs: Option<RootFS>,
}

#[derive(Debug, Deserialize)]
struct RootFS {
    #[serde(rename = "Type")]
    r#type: String,
    #[serde(rename = "Layers")]
    layers: Vec<String>,
}

#[derive(Parser)]
enum Cli {
    /// 根据目标机器上旧镜像的 inspect 信息, 创建指定镜像 tarball 的增量文件
    Delta {
        /// 指定镜像 tarball 路径
        tar_path: PathBuf,
        /// 旧镜像的 inspect 信息路径
        inspect_path: PathBuf,
    },
    /// 基于旧镜像 tarball, 使用增量文件进行修补
    Patch {
        /// 旧镜像 tarball 路径
        tar_path: PathBuf,
        /// 增量文件路径
        delta_path: PathBuf,
    },
}

fn get_manifest_layers_from_tarball(tar_path: &Path) -> Result<Vec<String>> {
    let file = File::open(tar_path)?;
    let mut archive = Archive::new(file);

    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.path()? == Path::new("manifest.json") {
            let mut content = String::new();
            entry.read_to_string(&mut content)?;
            let manifest: Vec<ManifestEntry> = serde_json::from_str(&content)
                .expect("manifest.json 解析失败");

            let mut layers = Vec::new();
            for image in manifest {
                for layer in image.layers {
                    layers.push(Path::new(&layer).file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string());
                }
            }
            return Ok(layers);
        }
    }

    anyhow::bail!("镜像 tarball 中不存在 manifest.json");
}

fn get_missing_layers_from_tarball(tar_path: &Path) -> Result<Vec<String>> {
    let file = File::open(tar_path)?;
    let mut archive = Archive::new(file);

    let mut blob_layers = HashSet::new();
    for entry in archive.entries()? {
        let entry = entry?;
        let path = entry.path()?;
        if let Some(path_str) = path.to_str() {
            if path_str.starts_with("blobs/sha256/") {
                if let Some(filename) = path.file_name() {
                    blob_layers.insert(filename.to_string_lossy().to_string());
                }
            }
        }
    }

    debug!("blob 层: {:?}", blob_layers);

    let manifest_layers: HashSet<String> = get_manifest_layers_from_tarball(tar_path)?
        .into_iter()
        .collect();

    debug!("manifest 层: {:?}", manifest_layers);

    let missing: Vec<String> = manifest_layers
        .difference(&blob_layers)
        .cloned()
        .collect();

    Ok(missing)
}

fn get_layers_from_inspect(inspect_path: &Path) -> Result<Vec<String>> {
    let content = fs::read_to_string(inspect_path)?;
    let inspect: Vec<InspectEntry> = serde_json::from_str(&content)?;

    let mut layers = Vec::new();
    for image in inspect {
        let image_id = image.id
            .strip_prefix("sha256:")
            .unwrap_or(&image.id)
            .chars()
            .take(12)
            .collect::<String>();

        if let Some(rootfs) = image.rootfs {
            if rootfs.r#type != "layers" {
                warn!("inspect 文件中镜像 {} 的 RootFS 类型不为 layers", image_id);
                continue;
            }

            for layer in rootfs.layers {
                let layer_id = layer
                    .strip_prefix("sha256:")
                    .unwrap_or(&layer)
                    .to_string();
                layers.push(layer_id);
            }
        } else {
            warn!("inspect 文件中镜像 {} 不存在 RootFS 字段", image_id);
        }
    }

    Ok(layers)
}

fn delta(tar_path: &Path, inspect_path: &Path) -> Result<()> {
    let tar_layers: HashSet<String> = get_manifest_layers_from_tarball(tar_path)?
        .into_iter()
        .collect();
    debug!("tarball 层: {:?}", tar_layers);
    info!("镜像 tarball 中共有 {} 层", tar_layers.len());

    let inspect_layers: HashSet<String> = get_layers_from_inspect(inspect_path)?
        .into_iter()
        .collect();
    debug!("inspect 层: {:?}", inspect_layers);
    info!("inspect 文件中镜像共有 {} 层", inspect_layers.len());

    let shared_layers: HashSet<String> = tar_layers
        .intersection(&inspect_layers)
        .cloned()
        .collect();
    debug!("共享层: {:?}", shared_layers);
    info!("可共享 {} 层", shared_layers.len());

    let shared_layers_path: HashSet<String> = shared_layers
        .iter()
        .map(|layer| format!("blobs/sha256/{}", layer))
        .collect();

    let delta_tar = tar_path.with_extension("delta");

    let old_file = File::open(tar_path)?;
    let mut old_archive = Archive::new(old_file);

    let new_file = File::create(&delta_tar)?;
    let mut new_builder = Builder::new(new_file);

    info!("开始生成增量文件");
    for entry_result in old_archive.entries()? {
        let mut entry = entry_result?;
        let path = entry.path()?.to_path_buf();
        let path_str = path.to_string_lossy().to_string();

        if shared_layers_path.contains(&path_str) {
            debug!("跳过 {}", path_str);
            continue;
        }

        // 创建新的entry并获取header和path
        let mut buffer = Vec::new();
        entry.read_to_end(&mut buffer)?;
        let mut new_entry = tar::Header::new_gnu();
        new_entry.set_size(buffer.len() as u64);
        new_entry.set_cksum();
        new_builder.append_data(&mut new_entry, &path, &buffer[..])?;
    }

    new_builder.finish()?;

    info!("增量文件生成完毕，保存在 {}", delta_tar.display());
    Ok(())
}

fn patch(tar_path: &Path, delta_path: &Path) -> Result<()> {
    let tar_layers: HashSet<String> = get_manifest_layers_from_tarball(tar_path)?
        .into_iter()
        .collect();
    debug!("tarball 层: {:?}", tar_layers);
    info!("镜像 tarball 中共有 {} 层", tar_layers.len());

    let missing_layers: HashSet<String> = get_missing_layers_from_tarball(delta_path)?
        .into_iter()
        .collect();
    debug!("增量文件中缺少层: {:?}", missing_layers);
    info!("增量文件中缺少 {} 层", missing_layers.len());

    let layer_not_found: HashSet<String> = missing_layers
        .difference(&tar_layers)
        .cloned()
        .collect();

    if !layer_not_found.is_empty() {
        debug!("tarball 缺少层: {:?}", layer_not_found);
        error!("tarball 中缺少 {} 层, 无法修补", layer_not_found.len());
        return Ok(());
    }

    info!("开始修补镜像 tarball");
    let new_tar_path = delta_path.with_extension("tar");

    // 复制delta文件到新tarball
    fs::copy(delta_path, &new_tar_path)?;

    // 以追加模式打开新文件
    let mut new_file = fs::OpenOptions::new()
        .append(true)
        .open(&new_tar_path)?;

    for missing_layer in missing_layers {
        debug!("添加 {}", missing_layer);
        let layer_path = format!("blobs/sha256/{}", missing_layer);

        // 重新打开旧文件以查找特定条目
        let old_file = File::open(tar_path)?;
        let mut old_archive = Archive::new(old_file);

        let mut found = false;
        for entry_result in old_archive.entries()? {
            let mut entry = entry_result?;
            let entry_path = entry.path()?;

            if entry_path == Path::new(&layer_path) {
                let mut buffer = Vec::new();
                entry.read_to_end(&mut buffer)?;

                // 将找到的条目添加到新文件
                new_file.write_all(&buffer)?;
                found = true;
                break;
            }
        }

        if !found {
            anyhow::bail!("在旧tarball中未找到层: {}", layer_path);
        }
    }

    info!("镜像 tarball 修补完毕，保存在 {}", new_tar_path.display());
    Ok(())
}

fn main() -> Result<()> {
    unsafe { env::set_var("RUST_LOG", "info"); }
    env_logger::init();

    match Cli::parse() {
        Cli::Delta { tar_path, inspect_path } => delta(&tar_path, &inspect_path),
        Cli::Patch { tar_path, delta_path } => patch(&tar_path, &delta_path),
    }
}
