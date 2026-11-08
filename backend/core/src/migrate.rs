//! One-time move of what was saved under oBot's old name, "openbot": the data folder, the chat
//! database, the Bots' folders, Claude Code's chat history for those folders and "Always allow"
//! rules for oBot's own tools. Each step runs only if it hasn't happened yet. If a folder can't be
//! moved (for example a program has a file open in it), the old place keeps being used, so nothing
//! is lost, and the move is tried again at the next start.

use crate::config::Config;
use crate::db::Db;
use std::path::{Path, PathBuf};

/// The data folder: `<base>/oBot`, after moving `<base>/openbot` there if needed.
pub fn data_dir(base: &Path) -> PathBuf {
    let (old, new) = (base.join("openbot"), base.join("oBot"));
    if new.exists() || !old.exists() {
        return new;
    }
    match std::fs::rename(&old, &new) {
        Ok(()) => {
            move_claude_history(&old, &new);
            new
        }
        Err(e) => {
            tracing::warn!("couldn't rename {} to {}: {e}; using the old folder", old.display(), new.display());
            old
        }
    }
}

/// The chat database: `obot.db`, after renaming `openbot.db` (and its -wal / -shm files) if needed.
pub fn db_file(data_dir: &Path) -> PathBuf {
    let (old, new) = (data_dir.join("openbot.db"), data_dir.join("obot.db"));
    if new.exists() || !old.exists() {
        return new;
    }
    if let Err(e) = std::fs::rename(&old, &new) {
        tracing::warn!("couldn't rename {}: {e}; using the old name", old.display());
        return old;
    }
    for ext in ["-wal", "-shm"] {
        let from = data_dir.join(format!("openbot.db{ext}"));
        if from.exists() {
            let _ = std::fs::rename(&from, data_dir.join(format!("obot.db{ext}")));
        }
    }
    new
}

/// Move the Bots' folders from `~/OpenBot` to `~/oBot` (when the old default is in use), point
/// each Bot at its new folder, and rename "Always allow" rules for oBot's tools.
pub fn bots_and_rules(cfg: &mut Config, config_path: &Path, db: &Db) {
    let mut changed = false;
    let rules: Vec<String> = cfg.allow_rules.iter().map(|r| r.replace("mcp__openbot__", "mcp__obot__")).collect();
    if rules != cfg.allow_rules {
        cfg.allow_rules = rules;
        changed = true;
    }
    if let Some(home) = dirs::home_dir() {
        let (old, new) = (home.join("OpenBot"), home.join("oBot"));
        if Path::new(&cfg.workspace_root) == old && !new.exists() {
            let moved = !old.exists() || std::fs::rename(&old, &new).is_ok();
            if moved {
                cfg.workspace_root = new.to_string_lossy().into_owned();
                changed = true;
                repoint_bots(db, &old, &new);
                move_claude_history(&old, &new);
            } else {
                tracing::warn!("couldn't rename {} to {}; Bots keep using the old folder", old.display(), new.display());
            }
        }
    }
    if changed {
        if let Err(e) = cfg.save(config_path) {
            tracing::warn!("couldn't save settings after renaming: {e}");
        }
    }
}

fn repoint_bots(db: &Db, old: &Path, new: &Path) {
    let Ok(bots) = db.list_bots() else { return };
    for mut b in bots {
        if let Ok(rest) = Path::new(&b.workdir).strip_prefix(old) {
            b.workdir = new.join(rest).to_string_lossy().into_owned();
            if let Err(e) = db.upsert_bot(&b) {
                tracing::warn!("couldn't update {}'s folder: {e}", b.name);
            }
        }
    }
}

/// Claude Code keeps each folder's chat history in `~/.claude/projects/<folder path with every
/// character that isn't a letter or digit replaced by "-">`. Rename the history of `old` and of
/// every folder inside it, so Bots continue their conversations in the moved folders.
fn move_claude_history(old: &Path, new: &Path) {
    let Some(projects) = dirs::home_dir().map(|h| h.join(".claude").join("projects")) else { return };
    let (old_key, new_key) = (history_key(old), history_key(new));
    let Ok(entries) = std::fs::read_dir(&projects) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix(&old_key) else { continue };
        if !(rest.is_empty() || rest.starts_with('-')) {
            continue;
        }
        let to = projects.join(format!("{new_key}{rest}"));
        if !to.exists() {
            if let Err(err) = std::fs::rename(e.path(), &to) {
                tracing::warn!("couldn't move Claude Code history {name}: {err}");
            }
        }
    }
}

fn history_key(p: &Path) -> String {
    p.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_keys_match_claude_code() {
        assert_eq!(history_key(Path::new(r"C:\Users\seane\OpenBot\Food - Fitness")), "C--Users-seane-OpenBot-Food---Fitness");
        assert_eq!(history_key(Path::new("/Users/me/oBot/School Bot")), "-Users-me-oBot-School-Bot");
    }

    #[test]
    fn old_data_moves_once() {
        let base = std::env::temp_dir().join(format!("obot-migrate-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("openbot")).unwrap();
        std::fs::write(base.join("openbot").join("openbot.db"), "db").unwrap();
        std::fs::write(base.join("openbot").join("openbot.db-wal"), "wal").unwrap();
        let dir = data_dir(&base);
        assert_eq!(dir, base.join("oBot"));
        assert!(!base.join("openbot").exists());
        assert_eq!(db_file(&dir), dir.join("obot.db"));
        assert_eq!(std::fs::read_to_string(dir.join("obot.db")).unwrap(), "db");
        assert_eq!(std::fs::read_to_string(dir.join("obot.db-wal")).unwrap(), "wal");
        // Second start: nothing left to move.
        assert_eq!(data_dir(&base), base.join("oBot"));
        assert_eq!(db_file(&dir), dir.join("obot.db"));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
