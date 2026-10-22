//! La voce "Apri in WinFiles" nel menu corto di Esplora file di Windows 11 (ADR 0022).
//!
//! Windows 11 mette nel menu corto solo i comandi di un pacchetto firmato: il pacchetto
//! (`installer/menu-win11/AppxManifest.xml`) dichiara questa DLL come server COM della classe
//! [`CLSID`], ed Esplora file la carica per sapere il titolo, l'icona e cosa fare al clic. Al clic
//! si avvia `WinFiles.exe --aggiungi` con tutti i file selezionati in una volta sola.
//!
//! Qui sta solo ciò che si prova anche fuori da Windows; la parte COM è in `com`.

#[cfg(windows)]
mod com;

/// Il titolo della voce nel menu.
pub const TITLE: &str = "Apri in WinFiles";

/// La classe COM della voce: la stessa in `AppxManifest.xml`. Non cambia mai.
pub const CLSID: &str = "6E2C9B4A-3D71-4F85-A0C2-91B7E5D4F318";

/// Il tetto di una riga di comando di Windows è 32 767 caratteri: si resta ben sotto.
pub const MAX_COMMAND: usize = 30_000;

/// Divide i percorsi in gruppi che stanno in una riga di comando; ogni gruppo è un avvio di
/// WinFiles, e gli avvii dopo il primo mandano i file alla stessa finestra.
pub fn batches(paths: &[String], limit: usize) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut len: usize = 0;
    for p in paths {
        // Le virgolette e lo spazio che separano gli argomenti.
        let cost = p.len().saturating_add(3);
        if !current.is_empty() && len.saturating_add(cost) > limit {
            out.push(std::mem::take(&mut current));
            len = 0;
        }
        current.push(p.clone());
        len = len.saturating_add(cost);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Il programma da avviare, dalla cartella che l'installer scrive nel registro.
pub fn exe_in(folder: &str) -> std::path::PathBuf {
    std::path::Path::new(folder.trim_end_matches(['\\', '/'])).join("WinFiles.exe")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pochi_file_vanno_in_un_solo_avvio() {
        let paths: Vec<String> = (0..20).map(|i| format!("C:\\Lavori\\file {i}.pdf")).collect();
        assert_eq!(batches(&paths, MAX_COMMAND), vec![paths]);
    }

    #[test]
    fn tanti_file_si_dividono_senza_perderne_nessuno() {
        let paths: Vec<String> = (0..3000)
            .map(|i| format!("C:\\Lavori\\Cliente\\file {i}.pdf"))
            .collect();
        let groups = batches(&paths, MAX_COMMAND);
        assert!(groups.len() > 1);
        for g in &groups {
            assert!(g.iter().map(|p| p.len() + 3).sum::<usize>() <= MAX_COMMAND);
        }
        assert_eq!(groups.concat(), paths);
    }

    #[test]
    fn un_percorso_lunghissimo_ha_il_suo_avvio() {
        let long = "x".repeat(40_000);
        let paths = vec!["a".to_string(), long.clone(), "b".to_string()];
        assert_eq!(
            batches(&paths, MAX_COMMAND),
            vec![vec!["a".to_string()], vec![long], vec!["b".to_string()]]
        );
        assert!(batches(&[], MAX_COMMAND).is_empty());
    }

    #[test]
    fn il_programma_sta_nella_cartella_dell_installer() {
        assert_eq!(
            exe_in("C:\\Users\\m\\AppData\\Local\\Programs\\WinFiles\\"),
            std::path::Path::new("C:\\Users\\m\\AppData\\Local\\Programs\\WinFiles").join("WinFiles.exe")
        );
    }

    #[test]
    fn la_classe_e_quella_del_pacchetto() {
        let manifest = include_str!("../../../installer/menu-win11/AppxManifest.xml");
        assert_eq!(manifest.matches(CLSID).count(), 3);
    }
}
