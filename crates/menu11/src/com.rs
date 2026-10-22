//! La parte COM: la classe che Esplora file chiede alla DLL e il comando del menu.
//!
//! Ogni blocco `unsafe` è una chiamata a Windows o una scrittura nei puntatori che Windows passa,
//! controllati prima di usarli. Nessun errore arriva a Esplora file come panic: si risponde con
//! un HRESULT e, al peggio, la voce non fa nulla.

use core::ffi::c_void;
use std::process::Command;

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_NOTIMPL, E_POINTER, S_FALSE,
};
use windows::Win32::System::Com::{CoTaskMemFree, IBindCtx, IClassFactory, IClassFactory_Impl};
use windows::Win32::UI::Shell::{
    ECF_DEFAULT, ECS_ENABLED, IEnumExplorerCommand, IExplorerCommand, IExplorerCommand_Impl, IShellItemArray,
    SHStrDupW, SIGDN_FILESYSPATH,
};
use windows_core::{BOOL, GUID, HRESULT, HSTRING, IUnknown, Interface, PWSTR, Ref, Result, implement};

fn clsid() -> GUID {
    GUID::try_from(crate::CLSID).unwrap_or_default()
}

/// La cartella di WinFiles che l'installer scrive in `HKCU\Software\WinFiles`.
fn exe() -> Option<std::path::PathBuf> {
    let key = windows_registry::CURRENT_USER.open("Software\\WinFiles").ok()?;
    let folder = key.get_string("Cartella").ok()?;
    Some(crate::exe_in(&folder))
}

/// I percorsi dei file selezionati; quelli che non sono file su disco (Rete, Cestino…) si saltano.
fn paths(items: &IShellItemArray) -> Vec<String> {
    let mut out = Vec::new();
    // SAFETY: `items` è un'interfaccia valida passata da Esplora file per tutta la chiamata.
    let count = unsafe { items.GetCount() }.unwrap_or(0);
    for i in 0..count {
        // SAFETY: indice dentro il conteggio appena letto.
        let Ok(item) = (unsafe { items.GetItemAt(i) }) else {
            continue;
        };
        // SAFETY: GetDisplayName restituisce una stringa allocata con CoTaskMemAlloc, che si
        // copia e poi si libera una volta sola.
        unsafe {
            if let Ok(name) = item.GetDisplayName(SIGDN_FILESYSPATH) {
                if let Ok(s) = name.to_string() {
                    out.push(s);
                }
                CoTaskMemFree(Some(name.0 as *const c_void));
            }
        }
    }
    out
}

/// Una stringa per Esplora file, che la libera lui.
fn shell_string(s: &str) -> Result<PWSTR> {
    // SAFETY: SHStrDupW copia la stringa in memoria di COM.
    unsafe { SHStrDupW(&HSTRING::from(s)) }
}

#[implement(IExplorerCommand)]
struct OpenCommand;

impl IExplorerCommand_Impl for OpenCommand_Impl {
    fn GetTitle(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        shell_string(crate::TITLE)
    }

    fn GetIcon(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        let exe = exe().ok_or(windows_core::Error::from(E_NOTIMPL))?;
        shell_string(&format!("{},0", exe.display()))
    }

    fn GetToolTip(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> Result<GUID> {
        Ok(clsid())
    }

    fn GetState(&self, _items: Ref<IShellItemArray>, _slow: BOOL) -> Result<u32> {
        Ok(ECS_ENABLED.0 as u32)
    }

    fn Invoke(&self, items: Ref<IShellItemArray>, _bind: Ref<IBindCtx>) -> Result<()> {
        let items = items.ok()?;
        let exe = exe().ok_or(windows_core::Error::from(E_NOTIMPL))?;
        for group in crate::batches(&paths(items), crate::MAX_COMMAND) {
            // Il primo avvio apre la finestra (se non c'è già), gli altri le mandano i file.
            let _ = Command::new(&exe).arg("--aggiungi").args(group).spawn();
        }
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
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
        let command: IExplorerCommand = OpenCommand.into();
        // SAFETY: `riid` e `object` sono puntatori validi controllati sopra.
        unsafe { command.query(riid, object) }.ok()
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

/// La DLL resta caricata finché Esplora file è aperto: costa pochi kB.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}
