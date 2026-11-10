//! Le anteprime di WinFiles davanti a quelle degli altri programmi (ADR 0036).
//!
//! Windows cerca il gestore delle anteprime prima nel ProgID del file (il programma che lo apre,
//! per esempio `Adobe.Illustrator.EPS` o `Photoshop.Image`) e solo dopo nell'estensione, dove lo
//! registra l'installer. Illustrator, Photoshop e i programmi CAD mettono il loro gestore nel
//! ProgID, e spesso non dà niente sui PC di oggi: allora si vede l'icona e non l'anteprima.
//! `DllRegisterServer` (che WinFiles chiama all'avvio con `regsvr32 /s`) scrive per l'utente
//! (HKCU) il gestore di WinFiles anche nei ProgID dei tipi che conosce e ricorda quali ha toccato;
//! `DllUnregisterServer` (alla disinstallazione) li toglie. Il programma che apre i file non
//! cambia.

use windows::Win32::Foundation::{ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, KEY_READ, REG_SZ, RRF_RT_REG_SZ, RegCloseKey,
    RegDeleteKeyValueW, RegDeleteKeyW, RegGetValueW, RegOpenKeyExW, RegSetKeyValueW,
};
use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
use windows_core::{HRESULT, HSTRING, PCWSTR};

/// La chiave del gestore delle anteprime (IThumbnailProvider) sotto ShellEx.
pub(crate) const THUMB_KEY: &str = r"ShellEx\{e357fccd-a995-4576-b01f-234630154e96}";
/// Dove si ricordano i ProgID toccati.
const STATE_KEY: &str = r"Software\WinFiles";
const STATE_VALUE: &str = "AnteprimeProgId";

/// Un valore di testo del registro, se c'è.
pub(crate) fn get(root: HKEY, key: &str, value: Option<&str>) -> Option<String> {
    let key = HSTRING::from(key);
    let name = value.map(HSTRING::from);
    let name_ptr = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
    let mut buf = [0u16; 512];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: `buf` è lungo `len` byte; Windows scrive al massimo tanto.
    let err = unsafe {
        RegGetValueW(
            root,
            &key,
            name_ptr,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if err != ERROR_SUCCESS {
        return None;
    }
    let chars = (len as usize / 2).min(buf.len());
    let text = String::from_utf16_lossy(buf.get(..chars).unwrap_or_default());
    let text = text.trim_end_matches('\0').trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn set(key: &str, value: Option<&str>, data: &str) -> WIN32_ERROR {
    let key = HSTRING::from(key);
    let name = value.map(HSTRING::from);
    let name_ptr = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
    let wide: Vec<u16> = data.encode_utf16().chain(Some(0)).collect();
    // SAFETY: `wide` termina con lo zero ed è lungo quanto dichiarato.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &key,
            name_ptr,
            REG_SZ.0,
            Some(wide.as_ptr().cast()),
            (wide.len() * 2) as u32,
        )
    }
}

pub(crate) fn exists(root: HKEY, key: &str) -> bool {
    let mut h = HKEY::default();
    // SAFETY: `h` riceve la chiave aperta, chiusa subito dopo.
    unsafe {
        if RegOpenKeyExW(root, &HSTRING::from(key), None, KEY_READ, &mut h) != ERROR_SUCCESS {
            return false;
        }
        let _ = RegCloseKey(h);
    }
    true
}

/// I ProgID di un'estensione: quello scelto dall'utente e quello dell'estensione.
pub(crate) fn prog_ids(ext: &str) -> Vec<String> {
    let choice = get(
        HKEY_CURRENT_USER,
        &format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.{ext}\UserChoice"),
        Some("ProgId"),
    );
    let class = get(HKEY_CLASSES_ROOT, &format!(".{ext}"), None);
    let mut out: Vec<String> = Vec::new();
    for p in [choice, class].into_iter().flatten() {
        if usable_prog_id(&p) && !out.iter().any(|o| o.eq_ignore_ascii_case(&p)) {
            out.push(p);
        }
    }
    out
}

/// Un ProgID in cui si può scrivere: un nome semplice, non un percorso.
pub(crate) fn usable_prog_id(p: &str) -> bool {
    !p.is_empty()
        && p.len() < 200
        && !p.contains('\\')
        && !p.starts_with('.')
        && !p.to_ascii_lowercase().starts_with("applications")
        && !p.to_ascii_lowercase().starts_with("appx")
        && p.chars().all(|c| !c.is_control())
}

/// L'elenco ricordato, unito ai nuovi senza doppioni.
pub(crate) fn merge(old: &str, new: &[String]) -> String {
    let mut all: Vec<&str> = old.split(';').filter(|s| !s.is_empty()).collect();
    for n in new {
        if !all.iter().any(|a| a.eq_ignore_ascii_case(n)) {
            all.push(n);
        }
    }
    all.join(";")
}

fn notify() {
    // SAFETY: notifica senza dati.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// Mette il gestore di WinFiles nei ProgID dei tipi conosciuti.
#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    let clsid = crate::CLSID;
    // La DLL è registrata dall'installer: senza, non si tocca niente.
    if !exists(
        HKEY_CURRENT_USER,
        &format!(r"Software\Classes\CLSID\{clsid}\InprocServer32"),
    ) {
        return HRESULT(0x8000_4005_u32 as i32);
    }
    let mut touched: Vec<String> = Vec::new();
    for ext in crate::EXTENSIONS {
        for p in prog_ids(ext) {
            if !exists(HKEY_CLASSES_ROOT, &p) {
                continue;
            }
            let key = format!(r"{p}\{THUMB_KEY}");
            let now = get(HKEY_CLASSES_ROOT, &key, None);
            if now.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(clsid)) {
                continue;
            }
            if set(&format!(r"Software\Classes\{key}"), None, clsid) == ERROR_SUCCESS {
                touched.push(p);
            }
        }
    }
    if !touched.is_empty() {
        let old = get(HKEY_CURRENT_USER, STATE_KEY, Some(STATE_VALUE)).unwrap_or_default();
        let _ = set(STATE_KEY, Some(STATE_VALUE), &merge(&old, &touched));
        notify();
    }
    HRESULT(0)
}

/// Toglie il gestore di WinFiles dai ProgID in cui l'aveva messo.
#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    let list = get(HKEY_CURRENT_USER, STATE_KEY, Some(STATE_VALUE)).unwrap_or_default();
    for p in list.split(';').filter(|p| usable_prog_id(p)) {
        let key = format!(r"Software\Classes\{p}\{THUMB_KEY}");
        let ours = get(HKEY_CURRENT_USER, &key, None).is_some_and(|v| v.eq_ignore_ascii_case(crate::CLSID));
        if ours {
            // SAFETY: chiavi dell'utente scritte da DllRegisterServer.
            unsafe {
                let _ = RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(key));
                let _ = RegDeleteKeyW(
                    HKEY_CURRENT_USER,
                    &HSTRING::from(format!(r"Software\Classes\{p}\ShellEx")),
                );
            }
        }
    }
    // SAFETY: valore dell'utente scritto da DllRegisterServer.
    unsafe {
        let _ = RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(STATE_KEY),
            &HSTRING::from(STATE_VALUE),
        );
    }
    notify();
    HRESULT(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solo_progid_semplici() {
        assert!(usable_prog_id("Adobe.Illustrator.EPS"));
        assert!(usable_prog_id("Photoshop.Image.26"));
        assert!(!usable_prog_id(""));
        assert!(!usable_prog_id(r"Applications\foo.exe"));
        assert!(!usable_prog_id("AppX1234abcd"));
        assert!(!usable_prog_id(".psd"));
    }

    #[test]
    fn l_elenco_si_unisce_senza_doppioni() {
        assert_eq!(merge("", &["A".into()]), "A");
        assert_eq!(merge("A;B", &["b".into(), "C".into()]), "A;B;C");
    }
}
