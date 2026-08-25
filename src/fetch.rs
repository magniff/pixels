//! Fetching weights from the hub, without an HTTP client.
//!
//! `curl` is already how this app opens a link, and it is on every machine the
//! app runs on. Shelling out to it costs one process and no dependency at all,
//! against a TLS stack and a certificate store — for a job the app does at most
//! twice in its life.
//!
//! Progress is the size of the partial file against the size the catalogue
//! says to expect, watched from the frame loop. Nothing here blocks: `curl`
//! runs on its own and is asked, once a frame, whether it has finished.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::settings::Weights;

pub struct Download {
    child: Child,
    /// Where it is being written, which is the final name plus `.part`.
    partial: PathBuf,
    /// Where it goes once it is whole.
    pub target: PathBuf,
    pub label: String,
    pub total: u64,
}

impl Download {
    /// Start fetching `what` into `dir`.
    pub fn start(what: &Weights, dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let target = dir.join(what.file);
        let partial = dir.join(format!("{}.part", what.file));
        // A short file under the final name is not weights — it is a fetch
        // that was cut off somewhere this app was not watching, by a curl run
        // by hand or a copy that ran out of disk. Moved aside rather than
        // refetched from nothing: `-C -` can finish what it started.
        if let Some(short) = short_file(&target, what.megabytes) {
            std::fs::rename(&target, &partial)
                .map_err(|e| format!("moving the partial file aside: {e}"))?;
            let _ = short;
        }
        // `-f` so a 404 is a failure rather than a file full of HTML, `-L` to
        // follow the hub's redirect to its CDN, `-C -` to pick up where an
        // interrupted attempt left off.
        let child = Command::new("curl")
            .args(["-fL", "-C", "-", "-o"])
            .arg(&partial)
            .arg(what.url)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("curl: {e}"))?;
        Ok(Self {
            child,
            partial,
            target,
            label: what.label.to_string(),
            total: what.megabytes * 1_000_000,
        })
    }

    /// How much has arrived, in bytes.
    pub fn bytes(&self) -> u64 {
        std::fs::metadata(&self.partial)
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// How far along, from zero to one.
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        (self.bytes() as f32 / self.total as f32).clamp(0.0, 1.0)
    }

    /// `None` while it is still going; the outcome once it is not.
    pub fn poll(&mut self) -> Option<Result<PathBuf, String>> {
        match self.child.try_wait() {
            Ok(None) => None,
            Ok(Some(status)) if status.success() => {
                // Renamed only once it is whole, so a half-fetched file is
                // never mistaken for weights.
                match std::fs::rename(&self.partial, &self.target) {
                    Ok(()) => Some(Ok(self.target.clone())),
                    Err(e) => Some(Err(format!("could not put it in place: {e}"))),
                }
            }
            Ok(Some(status)) => Some(Err(match status.code() {
                Some(22) => "the hub refused the request".to_string(),
                Some(6) => "could not reach the hub".to_string(),
                Some(code) => format!("curl gave up with code {code}"),
                None => "the download was interrupted".to_string(),
            })),
            Err(e) => Some(Err(format!("curl: {e}"))),
        }
    }

    /// Stop, and leave the partial file for `-C -` to resume from later.
    pub fn cancel(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The size on disk, when there is a file there and it is too small to be the
/// weights the catalogue describes.
///
/// Deliberately generous: quantisation and the hub's own rounding move the real
/// size around by a percent or two, and this is meant to catch a file that
/// stopped halfway, not to audit the catalogue.
pub fn short_file(path: &Path, megabytes: u64) -> Option<u64> {
    let have = std::fs::metadata(path).ok()?.len();
    let want = megabytes * 1_000_000;
    (have < want / 100 * 95).then_some(have)
}

/// A size in the unit a person would say it in.
pub fn megabytes(bytes: u64) -> String {
    format!("{} MB", bytes / 1_000_000)
}
