//! Bot skills: Claude Code skills saved in a Bot's folder (.claude/skills/<name>/SKILL.md).
//!
//! Bot runs load no settings files from the Bot's folder (a Bot could write one there to give
//! itself permissions, hooks or helper agents). So before each run oBot copies the skills into a
//! small plugin in oBot's own data folder and loads that with --plugin-dir. Each SKILL.md keeps
//! only its name, description and instructions: fields that change permissions or run commands
//! (allowed-tools, hooks, ...) are dropped.
//!
//! Skills are saved by the user (the Bot panel) and by the Bot itself (the save_skill tool), both
//! through `save`. Claude Code won't let a Bot write into .claude/ in some permission modes.

use std::path::{Path, PathBuf};

/// Plugin name. Claude Code shows plugin skills as "<plugin>:<skill>".
pub const PLUGIN: &str = "skills";
/// Frontmatter fields a skill keeps. Everything else is dropped.
const KEEP: [&str; 3] = ["name", "description", "when_to_use"];
const MAX_FILES: usize = 200;
const MAX_BYTES: u64 = 5 * 1024 * 1024;
const MAX_DEPTH: usize = 4;
/// A skill's description is listed in every message, so it stays one short line.
pub const MAX_DESCRIPTION: usize = 200;
/// Instructions are read only when the skill is used.
pub const MAX_BODY: usize = 20_000;
/// The text a skill had before its last save (one step of undo). Not copied into the plugin.
pub const PREVIOUS: &str = "SKILL.prev.md";

/// Where a Bot's skills live.
pub fn dir(workdir: &str) -> PathBuf {
    PathBuf::from(workdir).join(".claude").join("skills")
}

/// Skill names: lowercase letters, numbers and dashes, like weekly-report.
pub fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Save a skill's SKILL.md. The old text, if any, is kept as SKILL.prev.md. Ok(true) when it
/// replaced an existing skill. Err is a message for the user or the Bot.
pub fn save(workdir: &str, name: &str, description: &str, body: &str) -> Result<bool, String> {
    if !valid_name(name) {
        return Err("Use lowercase letters, numbers and dashes for the name, like weekly-report".into());
    }
    let body = body.trim();
    if body.is_empty() {
        return Err("Write the skill's instructions".into());
    }
    if body.chars().count() > MAX_BODY {
        return Err(format!("The instructions are too long (max {MAX_BODY} characters)"));
    }
    let desc = description.replace(['\n', '\r', '"'], " ");
    let desc = desc.trim();
    if desc.chars().count() > MAX_DESCRIPTION {
        return Err(format!("The description is too long (max {MAX_DESCRIPTION} characters): one line saying when to use it"));
    }
    let folder = dir(workdir).join(name);
    let file = folder.join("SKILL.md");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let old = std::fs::read_to_string(&file).ok();
    if let Some(old) = &old {
        std::fs::write(folder.join(PREVIOUS), old).map_err(|e| e.to_string())?;
    }
    std::fs::write(&file, format!("---\nname: {name}\ndescription: \"{desc}\"\n---\n\n{body}\n")).map_err(|e| e.to_string())?;
    Ok(old.is_some())
}

/// A skill's display name without the plugin prefix.
pub fn display_name(skill: &str) -> &str {
    skill.strip_prefix(PLUGIN).and_then(|s| s.strip_prefix(':')).unwrap_or(skill)
}

/// SKILL.md with only the safe frontmatter fields, and `name` set to the folder name.
fn clean_skill(name: &str, text: &str) -> String {
    let (front, body) = match text.strip_prefix("---").and_then(|r| r.find("\n---").map(|e| (&r[..e], &r[e + 4..]))) {
        Some((f, b)) => (f, b.trim_start_matches(['-']).trim_start()),
        None => ("", text),
    };
    let mut kept = vec![format!("name: {name}")];
    let mut keep = false;
    for line in front.lines() {
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if !indented {
            let key = line.split(':').next().unwrap_or("").trim();
            keep = !line.trim().is_empty() && key != "name" && KEEP.contains(&key);
        }
        if keep {
            kept.push(line.to_string());
        }
    }
    format!("---\n{}\n---\n\n{}", kept.join("\n"), body.trim())
}

/// Copy regular files (not links) from `from` to `to`, within the limits.
fn copy_files(from: &Path, to: &Path, depth: usize, files: &mut usize, bytes: &mut u64) -> std::io::Result<()> {
    for e in std::fs::read_dir(from)?.flatten() {
        let meta = std::fs::symlink_metadata(e.path())?;
        let dest = to.join(e.file_name());
        if meta.is_dir() && depth < MAX_DEPTH {
            std::fs::create_dir_all(&dest)?;
            copy_files(&e.path(), &dest, depth + 1, files, bytes)?;
        } else if meta.is_file() && e.file_name() != "SKILL.md" && !(depth == 1 && e.file_name() == PREVIOUS) {
            if *files >= MAX_FILES || *bytes + meta.len() > MAX_BYTES {
                continue;
            }
            *files += 1;
            *bytes += meta.len();
            std::fs::copy(e.path(), dest)?;
        }
    }
    Ok(())
}

/// Rebuild `<bot_dir>/skills-plugin` from the Bot's skills. None when the Bot has no skills.
pub fn build_plugin(workdir: &str, bot_dir: &Path) -> std::io::Result<Option<PathBuf>> {
    let out = bot_dir.join("skills-plugin");
    if out.exists() {
        std::fs::remove_dir_all(&out)?;
    }
    let Ok(rd) = std::fs::read_dir(dir(workdir)) else { return Ok(None) };
    let mut count = 0;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let valid = !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        let is_dir = std::fs::symlink_metadata(e.path()).map(|m| m.is_dir()).unwrap_or(false);
        let Ok(text) = std::fs::read_to_string(e.path().join("SKILL.md")) else { continue };
        if !valid || !is_dir || text.trim().is_empty() || text.len() as u64 > MAX_BYTES {
            continue;
        }
        let dest = out.join("skills").join(&name);
        std::fs::create_dir_all(&dest)?;
        std::fs::write(dest.join("SKILL.md"), clean_skill(&name, &text))?;
        let (mut files, mut bytes) = (0, 0);
        copy_files(&e.path(), &dest, 1, &mut files, &mut bytes)?;
        count += 1;
    }
    if count == 0 {
        let _ = std::fs::remove_dir_all(&out);
        return Ok(None);
    }
    std::fs::create_dir_all(out.join(".claude-plugin"))?;
    std::fs::write(
        out.join(".claude-plugin").join("plugin.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"name": PLUGIN, "description": "This Bot's saved skills (made by oBot)"}))?,
    )?;
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_unsafe_frontmatter() {
        let s = clean_skill(
            "report",
            "---\nname: other\ndescription: >\n  Weekly report\nallowed-tools: Bash(*)\nhooks:\n  Stop:\n    - x\n---\n\nDo the report.",
        );
        assert_eq!(s, "---\nname: report\ndescription: >\n  Weekly report\n---\n\nDo the report.");
        assert_eq!(clean_skill("a", "Just text"), "---\nname: a\n---\n\nJust text");
        assert_eq!(display_name("skills:report"), "report");
        assert_eq!(display_name("pdf"), "pdf");
    }

    #[test]
    fn builds_plugin_without_links() {
        let root = std::env::temp_dir().join(format!("ob-skills-{}", crate::util::new_id()));
        let work = root.join("work");
        let sk = dir(work.to_str().unwrap()).join("hello");
        std::fs::create_dir_all(sk.join("scripts")).unwrap();
        std::fs::write(sk.join("SKILL.md"), "---\nname: hello\nallowed-tools: Bash\n---\nHi").unwrap();
        std::fs::write(sk.join("scripts").join("run.sh"), "echo hi").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hosts", sk.join("link")).unwrap();
        let out = build_plugin(work.to_str().unwrap(), &root.join("bot")).unwrap().unwrap();
        let md = std::fs::read_to_string(out.join("skills/hello/SKILL.md")).unwrap();
        assert!(!md.contains("allowed-tools"));
        assert!(out.join("skills/hello/scripts/run.sh").exists());
        assert!(!out.join("skills/hello/link").exists());
        assert!(out.join(".claude-plugin/plugin.json").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn save_keeps_the_old_text_and_checks_its_input() {
        let root = std::env::temp_dir().join(format!("ob-skills-{}", crate::util::new_id()));
        let work = root.to_str().unwrap();
        assert_eq!(save(work, "Weekly Report", "x", "steps"), Err("Use lowercase letters, numbers and dashes for the name, like weekly-report".into()));
        assert!(save(work, "report", "x", "  ").is_err());
        assert!(save(work, "report", &"d".repeat(MAX_DESCRIPTION + 1), "steps").is_err());
        assert!(save(work, "report", "x", &"b".repeat(MAX_BODY + 1)).is_err());

        assert_eq!(save(work, "report", "Weekly report\n\"now\"", "1. Open the site"), Ok(false));
        let file = dir(work).join("report").join("SKILL.md");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "---\nname: report\ndescription: \"Weekly report  now\"\n---\n\n1. Open the site\n");
        assert_eq!(save(work, "report", "Weekly report", "1. Open the new site"), Ok(true));
        let prev = std::fs::read_to_string(dir(work).join("report").join(PREVIOUS)).unwrap();
        assert!(prev.contains("1. Open the site") && !prev.contains("new site"));
        // The old copy isn't loaded as part of the skill.
        let out = build_plugin(work, &root.join("bot")).unwrap().unwrap();
        assert!(out.join("skills/report/SKILL.md").exists());
        assert!(!out.join("skills/report").join(PREVIOUS).exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
