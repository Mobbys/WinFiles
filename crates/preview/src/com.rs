//! La parte COM: la classe che Esplora file chiede alla DLL per l'anteprima di un file.
//!
//! Windows passa il file come flusso di byte (`IInitializeWithStream`), poi chiede l'immagine
//! (`IThumbnailProvider`). Il flusso si legge in memoria, entro un tetto, e si scrive in un file
//! temporaneo con la stessa estensione, perché i lettori di WinFiles lavorano sui file.
//!
//! Ogni blocco `unsafe` è una chiamata a Windows o una scrittura nei puntatori che Windows passa,
//! controllati prima di usarli. Nessun errore arriva a Esplora file come panic: si risponde con un
//! HRESULT e Windows mostra l'icona normale.

use core::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_POINTER, E_UNEXPECTED, S_FALSE,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::System::Com::{
    CoTaskMemFree, IClassFactory, IClassFactory_Impl, IStream, STATFLAG_DEFAULT, STATSTG,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithStream, IInitializeWithStream_Impl};
use windows::Win32::UI::Shell::{IThumbnailProvider, IThumbnailProvider_Impl, WTS_ALPHATYPE, WTSAT_ARGB};
use windows_core::{BOOL, GUID, HRESULT, IUnknown, Interface, Ref, Result, implement};

fn clsid() -> GUID {
    GUID::try_from(crate::CLSID.trim_matches(['{', '}'])).unwrap_or_default()
}

/// Il file da disegnare: i byte e l'estensione (dal nome che dà il flusso).
#[implement(IInitializeWithStream, IThumbnailProvider)]
struct Preview {
    file: Mutex<Option<(Vec<u8>, String)>>,
}

/// L'estensione dal nome del flusso.
fn stream_ext(stream: &IStream) -> String {
    let mut stat = STATSTG::default();
    // SAFETY: `stat` è una struttura valida; il nome che Windows alloca si libera con
    // CoTaskMemFree.
    unsafe {
        if stream.Stat(&mut stat, STATFLAG_DEFAULT).is_err() || stat.pwcsName.is_null() {
            return String::new();
        }
        let name = stat.pwcsName.to_string().unwrap_or_default();
        CoTaskMemFree(Some(stat.pwcsName.0 as *const c_void));
        std::path::Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
    }
}

/// Tutti i byte del flusso, fino a [`crate::MAX_BYTES`].
fn read_all(stream: &IStream) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let mut read = 0u32;
        // SAFETY: `buf` è lungo quanto il numero di byte chiesti.
        let hr = unsafe {
            stream.Read(
                buf.as_mut_ptr().cast::<c_void>(),
                buf.len() as u32,
                Some(&mut read),
            )
        };
        if hr.is_err() {
            return Err(hr.into());
        }
        let got = buf.get(..read as usize).unwrap_or_default();
        if got.is_empty() {
            return Ok(out);
        }
        if (out.len() + got.len()) as u64 > crate::MAX_BYTES {
            return Err(E_FAIL.into());
        }
        out.extend_from_slice(got);
        if hr == S_FALSE {
            return Ok(out);
        }
    }
}

impl IInitializeWithStream_Impl for Preview_Impl {
    fn Initialize(&self, stream: Ref<IStream>, _mode: u32) -> Result<()> {
        let stream = stream.ok()?;
        let ext = stream_ext(stream);
        let bytes = read_all(stream).inspect_err(|e| log(&format!("«{ext}»: flusso non letto ({e})")))?;
        let mut file = self
            .file
            .lock()
            .map_err(|_| windows_core::Error::from(E_UNEXPECTED))?;
        *file = Some((bytes, ext));
        Ok(())
    }
}

/// Una riga nel registro degli errori delle anteprime (`%TEMP%\\winfiles-anteprime\\errori.txt`),
/// per capire perché un file non ha l'anteprima. Si ferma a 256 KB.
fn log(msg: &str) {
    use std::io::Write;
    let dir = std::env::temp_dir().join("winfiles-anteprime");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("errori.txt");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 256 * 1024) {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{msg}");
    }
}

/// Disegna l'anteprima dai byte, passando da un file temporaneo con la stessa estensione.
fn render(bytes: &[u8], ext: &str, side: u32) -> std::result::Result<winfiles_thumbs::Thumb, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join("winfiles-anteprime");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    // Il nome che dà Windows, se è di un tipo conosciuto; altrimenti il tipo dai primi byte.
    let ext = if crate::EXTENSIONS.contains(&ext) {
        ext
    } else {
        crate::sniff(bytes).ok_or_else(|| format!("tipo non riconosciuto (estensione «{ext}»)"))?
    };
    let path = dir.join(format!("{}-{n}.{ext}", std::process::id()));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    // Il processo delle anteprime di Windows (dllhost) ha una pila piccola (256 KB): i PDF
    // complessi, i PSD e gli AI la superano e il processo cade senza anteprima. Si disegna
    // allora in un thread con una pila grande, come fanno i lavoratori di WinFiles.
    let job = path.clone();
    let result = std::thread::Builder::new()
        .name("anteprima".into())
        .stack_size(RENDER_STACK)
        .spawn(move || {
            catch_unwind(AssertUnwindSafe(|| crate::thumbnail(&job, side)))
                .unwrap_or_else(|_| Err("errore interno".into()))
        })
        .map_err(|e| e.to_string())
        .and_then(|h| h.join().unwrap_or_else(|_| Err("errore interno".into())));
    let _ = std::fs::remove_file(&path);
    result
}

/// La pila del thread che disegna l'anteprima.
const RENDER_STACK: usize = 64 * 1024 * 1024;

/// L'immagine in una bitmap di Windows: dall'alto in basso, BGRA premoltiplicato.
fn bitmap(t: &winfiles_thumbs::Thumb) -> Result<HBITMAP> {
    let (w, h) = (t.width as i32, t.height as i32);
    if w <= 0 || h <= 0 || t.rgba.len() != (t.width as usize) * (t.height as usize) * 4 {
        return Err(E_FAIL.into());
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            // Negativa: le righe vanno dall'alto in basso, come in `rgba`.
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = core::ptr::null_mut();
    // SAFETY: `info` descrive una bitmap 32 bit di w × h; Windows alloca i pixel e li restituisce
    // in `bits`.
    let hbmp = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)? };
    if bits.is_null() {
        return Err(E_FAIL.into());
    }
    // SAFETY: Windows ha allocato w × h × 4 byte in `bits`, la stessa lunghezza di `t.rgba`.
    let out = unsafe { core::slice::from_raw_parts_mut(bits.cast::<u8>(), t.rgba.len()) };
    for (o, px) in out.chunks_exact_mut(4).zip(t.rgba.chunks_exact(4)) {
        if let ([ob, og, or, oa], [r, g, b, a]) = (o, px) {
            let k = u16::from(*a);
            *ob = ((u16::from(*b) * k) / 255) as u8;
            *og = ((u16::from(*g) * k) / 255) as u8;
            *or = ((u16::from(*r) * k) / 255) as u8;
            *oa = *a;
        }
    }
    Ok(hbmp)
}

impl IThumbnailProvider_Impl for Preview_Impl {
    fn GetThumbnail(&self, cx: u32, phbmp: *mut HBITMAP, pdwalpha: *mut WTS_ALPHATYPE) -> Result<()> {
        if phbmp.is_null() || pdwalpha.is_null() {
            return Err(E_POINTER.into());
        }
        let file = self
            .file
            .lock()
            .map_err(|_| windows_core::Error::from(E_UNEXPECTED))?
            .take()
            .ok_or_else(|| windows_core::Error::from(E_UNEXPECTED))?;
        let thumb = render(&file.0, &file.1, cx).map_err(|e| {
            log(&format!("«{}», {} byte: {e}", file.1, file.0.len()));
            windows_core::Error::from(E_FAIL)
        })?;
        let hbmp = bitmap(&thumb)?;
        // SAFETY: puntatori controllati sopra.
        unsafe {
            *phbmp = hbmp;
            *pdwalpha = WTSAT_ARGB;
        }
        Ok(())
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        riid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if object.is_null() || riid.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: `object` non è nullo; lo si azzera prima di tutto, come chiede COM.
        unsafe { *object = core::ptr::null_mut() };
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let preview: IThumbnailProvider = Preview {
            file: Mutex::new(None),
        }
        .into();
        // SAFETY: `riid` e `object` sono puntatori validi controllati sopra.
        unsafe { preview.query(riid, object) }.ok()
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// Esplora file chiede la fabbrica della classe [`crate::CLSID`].
///
/// # Safety
/// Chiamata da COM con puntatori validi o nulli; quelli nulli si rifiutano.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    if object.is_null() || riid.is_null() || rclsid.is_null() {
        return E_POINTER;
    }
    // SAFETY: puntatori controllati sopra.
    unsafe {
        *object = core::ptr::null_mut();
        if *rclsid != clsid() {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        factory.query(riid, object)
    }
}

/// Il processo delle anteprime (dllhost) la scarica quando ha finito.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW, HGDIOBJ};
    use windows::Win32::System::Com::STGM_READ;
    use windows::Win32::UI::Shell::SHCreateStreamOnFileEx;
    use windows_core::HSTRING;

    /// Come fa Esplora file: la fabbrica dalla DLL, la classe, il flusso del file, l'anteprima.
    #[test]
    fn esplora_file_riceve_l_anteprima_di_un_svg() {
        let path = std::env::temp_dir().join("winfiles-preview-com.svg");
        std::fs::write(
            &path,
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect width="200" height="100" fill="red"/></svg>"#,
        )
        .unwrap();
        unsafe {
            let mut raw = core::ptr::null_mut();
            let hr = DllGetClassObject(&clsid(), &IClassFactory::IID, &mut raw);
            assert!(hr.is_ok(), "{hr:?}");
            let factory = IClassFactory::from_raw(raw);
            let provider: IThumbnailProvider = factory.CreateInstance(None).unwrap();
            let init: IInitializeWithStream = provider.cast().unwrap();
            let stream =
                SHCreateStreamOnFileEx(&HSTRING::from(path.as_os_str()), STGM_READ.0, 0, false, None)
                    .unwrap();
            init.Initialize(&stream, STGM_READ.0).unwrap();
            let mut hbmp = HBITMAP::default();
            let mut alpha = WTS_ALPHATYPE::default();
            provider.GetThumbnail(256, &mut hbmp, &mut alpha).unwrap();
            assert_eq!(alpha, WTSAT_ARGB);
            let mut bm = BITMAP::default();
            let n = GetObjectW(
                HGDIOBJ(hbmp.0),
                core::mem::size_of::<BITMAP>() as i32,
                Some((&mut bm as *mut BITMAP).cast()),
            );
            assert!(n > 0);
            assert_eq!((bm.bmWidth, bm.bmHeight.abs()), (256, 128));
            let _ = DeleteObject(HGDIOBJ(hbmp.0));

            // Un'altra classe: rifiutata.
            let other = GUID::zeroed();
            assert_eq!(
                DllGetClassObject(&other, &IClassFactory::IID, &mut raw),
                CLASS_E_CLASSNOTAVAILABLE
            );
        }
    }
}
