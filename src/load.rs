//! STL と NAS をワールド座標のまま一つのスープに載せる。

use std::path::Path;

use anyhow::{bail, Result};

use crate::mesh::{bounds_to_soup, ParsedMesh, TriangleSoup};
use crate::nas;
use crate::open::{self, MeshKind};
use crate::stl;

/// 読めたメッシュを結合する。失敗したパスは警告に残し、三角形が一つも無ければエラー。
///
/// # Errors
///
/// どのファイルからも三角形が取れないとき。
///
/// # Examples
///
/// ```ignore
/// let (soup, warnings) = meshpad::load::load_paths(&["a.stl", "b.nas"])?;
/// let _ = (soup.triangle_count(), warnings.len());
/// ```
pub fn load_paths(paths: &[impl AsRef<Path>]) -> Result<(TriangleSoup, Vec<String>)> {
    let mut acc = ParsedMesh::empty();
    let mut warnings = Vec::new();
    for p in paths {
        let p = p.as_ref();
        match open::mesh_kind(p) {
            Some(MeshKind::Stl) => match stl::load_parsed(p) {
                Ok(mesh) => acc.absorb(mesh),
                Err(e) => warnings.push(format!("{}: {e}", p.display())),
            },
            Some(MeshKind::Nas) => match nas::load_nas(p) {
                Ok((mesh, nas_warnings)) => {
                    for w in nas_warnings {
                        warnings.push(format!("{}: {w}", p.display()));
                    }
                    acc.absorb(mesh);
                }
                Err(e) => warnings.push(format!("{}: {e}", p.display())),
            },
            None => warnings.push(format!("{}: unsupported extension", p.display())),
        }
    }
    if acc.positions.is_empty() {
        let detail = if warnings.is_empty() {
            "no triangles".into()
        } else {
            warnings.join("; ")
        };
        bail!("{detail}");
    }
    Ok((bounds_to_soup(acc.positions, acc.min, acc.max), warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn with_temp(test: impl FnOnce(&Path)) {
        let dir = std::env::temp_dir().join(format!(
            "meshpad-load-{}-{}",
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

    fn one_tri_ascii_stl() -> &'static str {
        "solid tri\n\
         facet normal 0 0 1\n\
         outer loop\n\
         vertex 0 0 0\n\
         vertex 1 0 0\n\
         vertex 0 1 0\n\
         endloop\n\
         endfacet\n\
         endsolid tri\n"
    }

    fn one_tri_nas() -> &'static str {
        "GRID           1              2.      0.      0.\n\
         GRID           2              3.      0.      0.\n\
         GRID           3              2.      1.      0.\n\
         CTRIA3         1       1       1       2       3\n"
    }

    #[test]
    fn mixes_stl_and_nas() {
        with_temp(|dir| {
            let stl = dir.join("a.stl");
            let nas = dir.join("b.nas");
            fs::write(&stl, one_tri_ascii_stl()).unwrap();
            fs::write(&nas, one_tri_nas()).unwrap();
            let (soup, warnings) = load_paths(&[stl, nas]).unwrap();
            assert!(warnings.is_empty());
            assert_eq!(soup.triangle_count(), 2);
        });
    }
}
