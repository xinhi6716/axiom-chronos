//! OS locks record live non-Codex council calls; leftover files never imply activity.
use std::{fs::{self, File, OpenOptions, TryLockError}, path::{Path,PathBuf}};

pub struct Member { file: Option<File>, path: PathBuf }
impl Member {
    pub fn start(home: &Path, name: &str) -> std::io::Result<Self> {
        if !matches!(name,"fable"|"argon") { return Err(std::io::Error::other("unknown council member")); }
        let folder = home.join("council-running"); fs::create_dir_all(&folder)?;
        let path = folder.join(format!("{}-{}-{name}.lock",std::process::id(),crate::base::unique()));
        let file = OpenOptions::new().read(true).write(true).create_new(true).open(&path)?;
        file.lock()?;
        Ok(Self { file:Some(file),path })
    }
}
impl Drop for Member {
    fn drop(&mut self) { drop(self.file.take()); let _=fs::remove_file(&self.path); }
}

pub fn models(home: &Path) -> std::io::Result<Vec<(String,u64)>> {
    let entries = match fs::read_dir(home.join("council-running")) {
        Ok(entries)=>entries, Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(vec![]), Err(e)=>return Err(e),
    };
    let mut models=std::collections::BTreeMap::new();
    for entry in entries {
        let entry=entry?; let name=entry.file_name(); let name=name.to_string_lossy();
        let model=if name.ends_with("-argon.lock") { super::ARGON_MODEL }
            else if name.ends_with("-fable.lock") { super::FABLE_MODEL } else { continue };
        let file=match OpenOptions::new().read(true).write(true).open(entry.path()) {
            Ok(file)=>file,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>continue,Err(e)=>return Err(e),
        };
        match file.try_lock() {
            Ok(())=>{},Err(TryLockError::WouldBlock)=>*models.entry(model.to_owned()).or_insert(0)+=1,
            Err(TryLockError::Error(e))=>return Err(e),
        }
    }
    Ok(models.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_only_locked_council_members_and_releases_on_exit() {
        let dir=tempfile::tempdir().unwrap();
        assert!(models(dir.path()).unwrap().is_empty());
        let argon=Member::start(dir.path(),"argon").unwrap();
        let fable=Member::start(dir.path(),"fable").unwrap();
        assert_eq!(models(dir.path()).unwrap(),vec![(super::super::FABLE_MODEL.into(),1),(super::super::ARGON_MODEL.into(),1)]);
        fs::write(dir.path().join("council-running/dead-argon.lock"),"").unwrap();
        drop(argon); drop(fable); assert!(models(dir.path()).unwrap().is_empty());
        fs::remove_file(dir.path().join("council-running/dead-argon.lock")).unwrap();
        fs::remove_dir(dir.path().join("council-running")).unwrap();
        fs::write(dir.path().join("council-running"),"bad").unwrap();
        assert!(models(dir.path()).is_err());
    }
}
