//! I lavori aperti di recente, per il menu File. Stanno in un piccolo file JSON nella cartella
//! delle impostazioni dell'utente (`%APPDATA%\WinFiles` su Windows); se non si può leggere o
//! scrivere, l'elenco è semplicemente vuoto.

use std::path::{Path, PathBuf};

/// Quanti lavori ricordare.
pub const MAX: usize = 10;
/// Tetto alla dimensione del file letto.
const MAX_BYTES: u64 = 256 * 1024;

/// Dove sta l'elenco.
fn store() -> Option<PathBuf> {
    crate::prefs::config_dir().map(|d| d.join("recenti.json"))
}

/// Legge l'elenco da `file`.
pub fn load_from(file: &Path) -> Vec<PathBuf> {
    let ok = std::fs::metadata(file).is_ok_and(|m| m.len() <= MAX_BYTES);
    if !ok {
        return Vec::new();
    }
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<PathBuf>>(&text).ok())
        .map(|mut v| {
            v.truncate(MAX);
            v
        })
        .unwrap_or_default()
}

/// Scrive l'elenco in `file`, creando la cartella se serve.
pub fn save_to(file: &Path, list: &[PathBuf]) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(list).map_err(std::io::Error::other)?;
    std::fs::write(file, json)
}

/// L'elenco dell'utente.
pub fn load() -> Vec<PathBuf> {
    store().map(|f| load_from(&f)).unwrap_or_default()
}

/// Mette `path` in cima all'elenco (senza doppioni) e lo salva.
pub fn remember(list: &mut Vec<PathBuf>, path: &Path) {
    push_front(list, path);
    if let Some(f) = store() {
        // Non riuscire a ricordare un lavoro recente non deve disturbare chi lavora.
        let _ = save_to(&f, list);
    }
}

/// Svuota l'elenco.
pub fn forget(list: &mut Vec<PathBuf>) {
    list.clear();
    if let Some(f) = store() {
        let _ = save_to(&f, list);
    }
}

fn push_front(list: &mut Vec<PathBuf>, path: &Path) {
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(MAX);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_cima_senza_doppioni_e_al_massimo_dieci() {
        let mut list = Vec::new();
        for i in 0..15 {
            push_front(&mut list, Path::new(&format!("/l/{i}.winfiles")));
        }
        push_front(&mut list, Path::new("/l/12.winfiles"));
        assert_eq!(list.len(), MAX);
        assert_eq!(list[0], Path::new("/l/12.winfiles"));
        assert_eq!(list.iter().filter(|p| p.ends_with("12.winfiles")).count(), 1);
    }

    #[test]
    fn salva_rilegge_e_ignora_i_file_rovinati() {
        let dir = std::env::temp_dir().join(format!("winfiles-recenti-{}", std::process::id()));
        let file = dir.join("sotto").join("recenti.json");
        let list = vec![PathBuf::from("/a.winfiles"), PathBuf::from("/b.winfiles")];
        save_to(&file, &list).unwrap();
        assert_eq!(load_from(&file), list);
        std::fs::write(&file, "non è json").unwrap();
        assert!(load_from(&file).is_empty());
        assert!(load_from(&dir.join("manca.json")).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
