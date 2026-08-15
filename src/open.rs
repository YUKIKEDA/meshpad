//! 開く操作の入力パスを、シーン用のファイル列へ展開する。
//!
//! ドロップ・ダイアログ・CLI の入口をここで揃える。フォルダは直下だけ見、再帰しない。

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// 1.0 でシーンに載せられる拡張子に対応する種類。
///
/// `.stl` は三角形、`.nas` / `.nastran` は外皮三角形として同じシーンに載せる。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshKind {
    /// `.stl`（大文字小文字は問わない）。
    Stl,
    /// `.nas` または `.nastran`。
    Nas,
}

/// パスの拡張子からメッシュ種類を返す。未知なら `None`。
///
/// 比較は ASCII の小文字化のみ。ディレクトリそのものには使わない。
///
/// # Examples
///
/// ```ignore
/// assert_eq!(mesh_kind(Path::new("a.STL")), Some(MeshKind::Stl));
/// assert!(mesh_kind(Path::new("notes.txt")).is_none());
/// ```
pub fn mesh_kind(path: &Path) -> Option<MeshKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "stl" => Some(MeshKind::Stl),
        "nas" | "nastran" => Some(MeshKind::Nas),
        _ => None,
    }
}

/// ドロップ・ダイアログ・CLI の入力を、シーンに載せるファイル列へ展開する。
///
/// ファイルはメッシュ拡張子だけ残す。未知の拡張子は警告してスキップする。
/// フォルダは直下の `.stl` / `.nas` / `.nastran` のみ（名前順）。入れ子は見ない。
/// フォルダ直下の未知ファイルは黙って飛ばす（ドロップ単位の未知ファイルだけ警告する）。
/// 同じファイルがフォルダ展開と単体指定で重なっても、先に出たパスだけ残す。
///
/// 戻り値の先頭がタイトル用。警告は呼び出し側がバーへ出す。
///
/// # Examples
///
/// ```ignore
/// let (files, warnings) = meshpad::open::expand_open_inputs(&[folder]);
/// let _ = (files.len(), warnings.len());
/// ```
pub fn expand_open_inputs(inputs: &[impl AsRef<Path>]) -> (Vec<PathBuf>, Vec<String>) {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut warnings = Vec::new();
    for input in inputs {
        let path = input.as_ref();
        match classify_existing(path) {
            PathClass::Missing => {
                warnings.push(format!("{}: not found", path.display()));
            }
            PathClass::Dir => match expand_dir(path) {
                Ok(kids) => {
                    if kids.is_empty() {
                        warnings.push(format!(
                            "{}: no .stl / .nas / .nastran in this folder",
                            path.display()
                        ));
                    } else {
                        for kid in kids {
                            push_unique(&mut files, &mut seen, kid);
                        }
                    }
                }
                Err(e) => warnings.push(format!("{}: {e}", path.display())),
            },
            PathClass::File => match mesh_kind(path) {
                Some(_) => push_unique(&mut files, &mut seen, path.to_path_buf()),
                None => warnings.push(format!("{}: unsupported extension", path.display())),
            },
        }
    }
    (files, warnings)
}

fn path_identity(path: &Path) -> OsString {
    std::fs::canonicalize(path)
        .map(|p| p.into_os_string())
        .unwrap_or_else(|_| path.as_os_str().to_os_string())
}

fn push_unique(files: &mut Vec<PathBuf>, seen: &mut HashSet<OsString>, path: PathBuf) {
    if seen.insert(path_identity(&path)) {
        files.push(path);
    }
}

enum PathClass {
    Missing,
    Dir,
    File,
}

fn classify_existing(path: &Path) -> PathClass {
    if !path.exists() {
        PathClass::Missing
    } else if path.is_dir() {
        PathClass::Dir
    } else {
        PathClass::File
    }
}

fn expand_dir(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut kids = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            continue;
        }
        if mesh_kind(&path).is_some() {
            kids.push(path);
        }
    }
    kids.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(kids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn with_temp(test: impl FnOnce(&Path)) {
        let dir = std::env::temp_dir().join(format!(
            "meshpad-open-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test(&dir)));
        let _ = fs::remove_dir_all(&dir);
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn mesh_kind_is_case_insensitive() {
        assert_eq!(mesh_kind(Path::new("a.STL")), Some(MeshKind::Stl));
        assert_eq!(mesh_kind(Path::new("b.Nastran")), Some(MeshKind::Nas));
        assert!(mesh_kind(Path::new("c.obj")).is_none());
    }

    #[test]
    fn keeps_stl_file() {
        with_temp(|dir| {
            let stl = dir.join("part.stl");
            touch(&stl);
            let (files, warnings) = expand_open_inputs(&[stl.as_path()]);
            assert_eq!(files, vec![stl]);
            assert!(warnings.is_empty());
        });
    }

    #[test]
    fn unknown_file_is_skipped_with_warning() {
        with_temp(|dir| {
            let txt = dir.join("notes.txt");
            touch(&txt);
            let (files, warnings) = expand_open_inputs(&[txt.as_path()]);
            assert!(files.is_empty());
            assert_eq!(warnings.len(), 1);
            assert!(warnings[0].contains("unsupported extension"));
        });
    }

    #[test]
    fn folder_takes_immediate_mesh_children_sorted() {
        with_temp(|dir| {
            touch(&dir.join("b.stl"));
            touch(&dir.join("a.NAS"));
            touch(&dir.join("skip.txt"));
            let nested = dir.join("nested");
            touch(&nested.join("hidden.stl"));
            let (files, warnings) = expand_open_inputs(&[dir.to_path_buf()]);
            assert!(warnings.is_empty());
            let names: Vec<_> = files
                .iter()
                .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
                .collect();
            assert_eq!(names, vec!["a.NAS", "b.stl"]);
        });
    }

    #[test]
    fn empty_folder_warns() {
        with_temp(|dir| {
            touch(&dir.join("readme.txt"));
            let (files, warnings) = expand_open_inputs(&[dir.to_path_buf()]);
            assert!(files.is_empty());
            assert_eq!(warnings.len(), 1);
            assert!(warnings[0].contains("no .stl"));
        });
    }

    #[test]
    fn missing_path_warns() {
        let (files, warnings) = expand_open_inputs(&[PathBuf::from("definitely-missing.stl")]);
        assert!(files.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("not found"));
    }

    #[test]
    fn file_then_folder_keeps_input_order() {
        with_temp(|dir| {
            let first = dir.join("first.stl");
            touch(&first);
            let folder = dir.join("pack");
            touch(&folder.join("inner.stl"));
            let (files, warnings) = expand_open_inputs(&[first.clone(), folder]);
            assert!(warnings.is_empty());
            assert_eq!(files.len(), 2);
            assert_eq!(files[0], first);
            assert_eq!(files[1].file_name().unwrap(), "inner.stl");
        });
    }

    #[test]
    fn folder_and_child_file_are_not_duplicated() {
        with_temp(|dir| {
            let stl = dir.join("part.stl");
            touch(&stl);
            let (files, warnings) = expand_open_inputs(&[stl.as_path(), dir]);
            assert!(warnings.is_empty());
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].file_name().unwrap(), "part.stl");
        });
    }

    #[test]
    fn same_folder_twice_is_not_duplicated() {
        with_temp(|dir| {
            touch(&dir.join("part.stl"));
            let (files, warnings) = expand_open_inputs(&[dir.to_path_buf(), dir.to_path_buf()]);
            assert!(warnings.is_empty());
            assert_eq!(files.len(), 1);
        });
    }
}
