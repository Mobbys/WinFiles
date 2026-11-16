//! La parte che parla con Windows: la lista «Apri con» di ogni tipo, i programmi cercati per nome,
//! le icone, l'avvio delle app dello Store. Ogni blocco `unsafe` è una chiamata COM o GDI con
//! argomenti validi, e ogni risorsa presa (stringhe, bitmap, DC) si libera nello stesso blocco.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    GetDIBits, GetObjectW, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    IDataObject,
};
use windows::Win32::UI::Shell::{
    ASSOC_FILTER_NONE, ASSOC_FILTER_RECOMMENDED, ApplicationActivationManager, BHID_DataObject,
    IApplicationActivationManager, IAssocHandler, IObjectWithAppUserModelID, IShellItem, IShellItemArray,
    IShellItemImageFactory, SHAssocEnumHandlers, SHCreateItemFromParsingName,
    SHCreateShellItemArrayFromShellItem, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows_core::{HSTRING, Interface, PWSTR, w};

use crate::types::{self, FAMILIES, TYPES};
use crate::{Found, Icon, Program};

/// Lato dell'icona chiesta a Windows (in pixel: basta per i pulsanti anche al 200%).
const ICON_SIDE: i32 = 48;
/// Né più grande di così, qualunque cosa risponda Windows.
const ICON_MAX: i32 = 256;
/// Programmi letti per ogni tipo al massimo.
const MAX_HANDLERS: usize = 64;

/// COM nel thread che chiama (le chiamate sono per thread). Se è già acceso, non cambia nulla.
fn com() {
    // SAFETY: nessun puntatore; il risultato dice solo se era già acceso.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
}

/// Una stringa che Windows ha allocato per noi: copiata e liberata.
fn take(p: PWSTR) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: `p` viene da Windows ed è una stringa terminata da zero, da liberare con CoTaskMemFree.
    let s = unsafe { p.to_string().ok() };
    // SAFETY: liberata una sola volta, non usata dopo.
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s
}

/// Un programma della lista «Apri con» di un'estensione.
#[derive(Clone, Debug)]
pub(crate) struct Handler {
    /// Il nome per Windows: il percorso dell'eseguibile per i programmi classici.
    pub name: String,
    /// Il nome da mostrare.
    pub ui_name: String,
    /// L'identità dell'app dello Store, se è una di quelle.
    pub app_id: Option<String>,
}

fn handler_list(ext: &str, recommended: bool) -> Vec<(Handler, IAssocHandler)> {
    com();
    let ext = format!(".{}", ext.trim_start_matches('.'));
    let filter = if recommended {
        ASSOC_FILTER_RECOMMENDED
    } else {
        ASSOC_FILTER_NONE
    };
    let mut out = Vec::new();
    // SAFETY: chiamate COM con argomenti validi; le stringhe le libera `take`.
    unsafe {
        let Ok(list) = SHAssocEnumHandlers(&HSTRING::from(ext), filter) else {
            return out;
        };
        while out.len() < MAX_HANDLERS {
            let mut one = [None];
            let mut got = 0u32;
            if list.Next(&mut one, Some(&mut got)).is_err() || got == 0 {
                break;
            }
            let [Some(h)] = one else { break };
            let Some(name) = h.GetName().ok().and_then(take) else {
                continue;
            };
            let ui_name = h.GetUIName().ok().and_then(take).unwrap_or_else(|| name.clone());
            let app_id = h
                .cast::<IObjectWithAppUserModelID>()
                .ok()
                .and_then(|o| o.GetAppID().ok())
                .and_then(take)
                .filter(|s| !s.is_empty());
            out.push((
                Handler {
                    name,
                    ui_name,
                    app_id,
                },
                h,
            ));
        }
    }
    out
}

/// I programmi che Windows propone per un'estensione (senza punto).
pub(crate) fn handlers(ext: &str) -> Vec<Handler> {
    handler_list(ext, true).into_iter().map(|(h, _)| h).collect()
}

fn is_exe(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".exe") && Path::new(name).is_absolute()
}

fn is_winfiles(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("winfiles.exe"))
}

/// Un percorso del registro, senza virgolette e con le variabili d'ambiente più comuni.
fn clean_path(raw: &str) -> PathBuf {
    let mut s = raw.trim().trim_matches('"').to_string();
    for var in [
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "SystemRoot",
        "windir",
        "LOCALAPPDATA",
    ] {
        let pat = format!("%{var}%").to_ascii_lowercase();
        if let Some(i) = s.to_ascii_lowercase().find(&pat)
            && let Ok(val) = std::env::var(var)
        {
            s.replace_range(i..i + pat.len(), &val);
        }
    }
    PathBuf::from(s)
}

/// L'eseguibile da «App Paths» del registro (dove gli installatori lo scrivono).
fn app_path(exe: &str) -> Option<PathBuf> {
    let key = format!(r"Software\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
    [windows_registry::CURRENT_USER, windows_registry::LOCAL_MACHINE]
        .iter()
        .filter_map(|root| root.open(&key).ok())
        .filter_map(|k| k.get_string("").ok())
        .map(|s| clean_path(&s))
        .find(|p| p.is_file())
}

/// Le cartelle dei programmi (64 e 32 bit).
fn program_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
        .collect();
    dirs.dedup();
    dirs
}

/// Cerca uno degli eseguibili dentro `dir`, fino a `depth` cartelle sotto.
fn find_exe(dir: &Path, exes: &[&str], depth: u32) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for e in entries.flatten().take(400) {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        if exes.contains(&name.as_str()) && path.is_file() {
            return Some(path);
        }
        if depth > 0 && path.is_dir() {
            subdirs.push(path);
        }
    }
    subdirs.into_iter().find_map(|d| find_exe(&d, exes, depth - 1))
}

/// Un programma Adobe nelle cartelle di Adobe: la versione più recente (il nome della cartella
/// porta l'anno, «Adobe Photoshop 2026»; a pari nome vince la data del file). Acrobat prima di
/// Reader.
fn adobe(prefix: &str, exes: &[&str]) -> Option<PathBuf> {
    let mut best: Option<(String, std::time::SystemTime, PathBuf)> = None;
    let prefix = prefix.to_ascii_lowercase();
    for root in program_dirs() {
        let Ok(entries) = std::fs::read_dir(root.join("Adobe")) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.to_ascii_lowercase().starts_with(&prefix) {
                continue;
            }
            let Some(exe) = find_exe(&e.path(), exes, 2) else {
                continue;
            };
            let when = exe
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            let rank = exes
                .iter()
                .position(|x| {
                    exe.file_name()
                        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(x))
                })
                .unwrap_or(exes.len())
                .min(9);
            let sort = format!("{}{name}", 9 - rank);
            if best.as_ref().is_none_or(|(n, t, _)| (&sort, when) > (n, *t)) {
                best = Some((sort, when, exe));
            }
        }
    }
    best.map(|(_, _, p)| p)
}

/// Gli altri programmi noti nelle cartelle dei programmi (CorelDRAW, Inkscape, GIMP, AutoCAD).
fn known_dir(exes: &[&str]) -> Option<PathBuf> {
    let hints = ["corel", "inkscape", "gimp", "autodesk"];
    program_dirs().into_iter().find_map(|root| {
        std::fs::read_dir(&root).ok()?.flatten().find_map(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if hints.iter().any(|h| name.starts_with(h)) {
                find_exe(&e.path(), exes, 3)
            } else {
                None
            }
        })
    })
}

fn family_found(f: &types::Family) -> Found {
    Found {
        key: f.key.into(),
        name: f.name.into(),
        label: f.label.into(),
        types: f.types.iter().map(|t| (*t).to_string()).collect(),
        ..Found::default()
    }
}

pub(crate) fn detect() -> Vec<Found> {
    let mut all: BTreeMap<String, Found> = BTreeMap::new();
    // 1. La lista «Apri con» di Windows, tipo per tipo.
    for t in TYPES {
        for ext in t.exts.iter().take(2) {
            for h in handlers(ext) {
                let exe = is_exe(&h.name).then(|| PathBuf::from(&h.name));
                if exe.as_deref().is_some_and(is_winfiles) {
                    continue;
                }
                let family = exe
                    .as_deref()
                    .and_then(|e| e.file_name())
                    .and_then(|n| types::family_of_exe(&n.to_string_lossy()))
                    .or_else(|| h.app_id.as_deref().and_then(types::family_of_app_id));
                let key = match (family, &h.app_id, &exe) {
                    (Some(f), _, _) => f.key.to_string(),
                    (None, Some(id), _) => format!("app:{id}"),
                    (None, None, Some(e)) => crate::exe_key(e),
                    (None, None, None) => format!("handler:{}", h.name),
                };
                let entry = all.entry(key.clone()).or_insert_with(|| match family {
                    Some(f) => family_found(f),
                    None => Found {
                        key,
                        name: h.ui_name.clone(),
                        label: h.ui_name.clone(),
                        ..Found::default()
                    },
                });
                entry.types.insert(t.key.to_string());
                if entry.exe.is_none() {
                    entry.exe = exe;
                }
                if entry.app_id.is_none() {
                    entry.app_id = h.app_id.clone();
                }
                if entry.exe.is_none() && entry.app_id.is_none() && entry.handler.is_none() {
                    entry.handler = Some(h.name.clone());
                }
            }
        }
    }
    // 2. I programmi noti cercati per nome: anche quando non sono registrati per quei tipi, e
    //    sempre la versione più recente.
    for f in FAMILIES {
        let exe = match f.adobe_dir {
            Some(d) => adobe(d, f.exes).or_else(|| f.exes.iter().find_map(|e| app_path(e))),
            None => f
                .exes
                .iter()
                .find_map(|e| app_path(e))
                .or_else(|| known_dir(f.exes)),
        };
        if let Some(exe) = exe {
            let entry = all.entry(f.key.into()).or_insert_with(|| family_found(f));
            entry.exe = Some(exe);
            entry.types.extend(f.types.iter().map(|t| (*t).to_string()));
        }
    }
    all.into_values().collect()
}

/// Da una bitmap di Windows a RGBA (alfa premoltiplicato, righe dall'alto).
fn to_rgba(hbmp: HBITMAP) -> Option<Icon> {
    let mut bm = BITMAP::default();
    // SAFETY: `bm` è grande quanto dichiarato.
    let n = unsafe {
        GetObjectW(
            HGDIOBJ(hbmp.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut BITMAP as *mut _),
        )
    };
    if n == 0 || bm.bmWidth <= 0 || bm.bmHeight == 0 {
        return None;
    }
    let (w, h) = (bm.bmWidth.min(ICON_MAX), bm.bmHeight.abs().min(ICON_MAX));
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..BITMAPINFOHEADER::default()
        },
        ..BITMAPINFO::default()
    };
    let len = usize::try_from(w)
        .ok()?
        .checked_mul(usize::try_from(h).ok()?)?
        .checked_mul(4)?;
    let mut buf = vec![0u8; len];
    // SAFETY: il DC si crea e si cancella qui; `buf` contiene `h` righe di `w` pixel a 32 bit.
    let lines = unsafe {
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(
            dc,
            hbmp,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        lines
    };
    if lines <= 0 {
        return None;
    }
    // Da BGRA a RGBA. Senza alfa (tutto 0) l'icona è opaca.
    let opaque = buf.chunks_exact(4).all(|p| p.get(3) == Some(&0));
    for p in buf.chunks_exact_mut(4) {
        p.swap(0, 2);
        if opaque && let Some(a) = p.get_mut(3) {
            *a = 255;
        }
    }
    Some(Icon {
        width: w as u32,
        height: h as u32,
        rgba: buf,
    })
}

/// L'icona di un elemento della shell: un eseguibile o `shell:AppsFolder\<app>`.
fn shell_icon(target: &str) -> Option<Icon> {
    com();
    // SAFETY: chiamate COM con argomenti validi; la bitmap si libera dopo la copia.
    unsafe {
        let f: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(target), None).ok()?;
        let hbmp = f
            .GetImage(
                SIZE {
                    cx: ICON_SIDE,
                    cy: ICON_SIDE,
                },
                SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
            )
            .ok()?;
        let icon = to_rgba(hbmp);
        let _ = DeleteObject(HGDIOBJ(hbmp.0));
        icon
    }
}

pub(crate) fn icon(p: &Program) -> Option<Icon> {
    if let Some(exe) = p.exe.as_deref().filter(|e| e.is_file())
        && let Some(i) = shell_icon(&exe.to_string_lossy())
    {
        return Some(i);
    }
    if let Some(id) = &p.app_id
        && let Some(i) = shell_icon(&format!(r"shell:AppsFolder\{id}"))
    {
        return Some(i);
    }
    p.handler.as_deref().filter(|h| is_exe(h)).and_then(shell_icon)
}

fn item(path: &Path) -> Result<IShellItem, String> {
    // SAFETY: chiamata della shell con un percorso valido.
    unsafe { SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None) }
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Apre i file con un'app dello Store, uno alla volta.
pub(crate) fn activate(app_id: &str, files: &[PathBuf]) -> Result<(), String> {
    com();
    // SAFETY: chiamate COM con argomenti validi.
    unsafe {
        let manager: IApplicationActivationManager =
            CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)
                .map_err(|e| e.to_string())?;
        for f in files {
            let array: IShellItemArray =
                SHCreateShellItemArrayFromShellItem(&item(f)?).map_err(|e| e.to_string())?;
            manager
                .ActivateForFile(&HSTRING::from(app_id), &array, w!("open"))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Apre i file con il programma che Windows elenca con questo nome in «Apri con».
pub(crate) fn invoke(name: &str, files: &[PathBuf]) -> Result<(), String> {
    for f in files {
        let ext = f
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        let handler = handler_list(&ext, false)
            .into_iter()
            .find(|(h, _)| h.name == name)
            .map(|(_, h)| h)
            .ok_or_else(|| format!("{name} non è più fra i programmi per i .{ext}"))?;
        // SAFETY: chiamate COM con argomenti validi.
        unsafe {
            let data: IDataObject = item(f)?
                .BindToHandler(None, &BHID_DataObject)
                .map_err(|e| e.to_string())?;
            handler.Invoke(&data).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
