//! Le ultime scelte dell'utente, ritrovate alla riapertura: posizione dei pannelli, tema, unità,
//! impaginazione, stampante e misura della finestra. Stanno in `impostazioni.json` nella cartella
//! delle impostazioni (`%APPDATA%\WinFiles` su Windows). Un file mancante, rovinato o di una
//! versione diversa non blocca nulla: si riparte dai valori predefiniti, campo per campo.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use craft_print::Duplex;
use craft_ui::docking::Layout;
use serde::{Deserialize, Serialize};
use winfiles_geom::{Calibration, Unit};
use winfiles_model::PrintSettings;

use crate::dock::PanelId;
use crate::theme::ThemeChoice;

/// Tetto alla dimensione del file letto.
const MAX_BYTES: u64 = 256 * 1024;
/// La finestra non si riapre più piccola di così (come `with_min_inner_size` in `main`).
pub const MIN_WINDOW: [f32; 2] = [800.0, 560.0];
/// Stampanti di cui si ricorda la correzione al massimo.
const MAX_CALIBRATIONS: usize = 64;
/// Né più grande di così, contro valori assurdi nel file.
const MAX_WINDOW: f32 = 16_000.0;

/// La cartella delle impostazioni dell'utente.
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.map(|b| b.join("WinFiles"))
}

fn store() -> Option<PathBuf> {
    config_dir().map(|d| d.join("impostazioni.json"))
}

/// Fronte e retro, come si scrive nel file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sides {
    #[default]
    One,
    LongEdge,
    ShortEdge,
}

impl Sides {
    pub fn from_duplex(d: Duplex) -> Self {
        match d {
            Duplex::LongEdge => Sides::LongEdge,
            Duplex::ShortEdge => Sides::ShortEdge,
            Duplex::Off => Sides::One,
        }
    }

    pub fn duplex(self) -> Duplex {
        match self {
            Sides::One => Duplex::Off,
            Sides::LongEdge => Duplex::LongEdge,
            Sides::ShortEdge => Duplex::ShortEdge,
        }
    }
}

/// Le scelte della finestra Stampa da ricordare. Le copie e i fogli no: si riparte sempre da una
/// copia di tutti i fogli, per non stampare per sbaglio dieci copie del lavoro dopo.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrintChoices {
    pub printer: Option<String>,
    pub sides: Sides,
    pub gray: bool,
    pub fit: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// I pannelli; `None` = la disposizione predefinita.
    pub dock: Option<Layout<PanelId>>,
    pub theme: ThemeChoice,
    pub unit: Unit,
    /// L'impaginazione di partenza per un lavoro nuovo (senza titolo, che è del lavoro).
    pub print: PrintSettings,
    pub printing: PrintChoices,
    /// La correzione di ogni stampante calibrata, per nome.
    pub calibrations: BTreeMap<String, Calibration>,
    /// I modelli di impaginazione salvati.
    pub presets: Vec<crate::presets::Preset>,
    /// Misura interna della finestra, in punti.
    pub window: Option<[f32; 2]>,
    pub maximized: bool,
    /// Spenta la casella "Includi i file non supportati": restano fuori dall'elenco.
    pub hide_unsupported: bool,
    /// Si è scelto di tenere la vecchia WinFile 4.9: non lo si chiede più.
    pub keep_legacy: bool,
}

impl Prefs {
    /// Toglie dal file letto ciò che non si può usare: una disposizione dei pannelli rovinata o
    /// senza l'anteprima, una finestra di misura assurda, un'impaginazione impossibile.
    fn sanitized(mut self) -> Self {
        let dock_ok = self
            .dock
            .as_ref()
            .is_some_and(|d| d.validate().is_ok() && d.contains(&PanelId::Canvas) && d.root.is_some());
        if !dock_ok {
            self.dock = None;
        }
        self.window = self
            .window
            .filter(|[w, h]| w.is_finite() && h.is_finite() && *w >= MIN_WINDOW[0] && *h >= MIN_WINDOW[1]);
        self.window = self.window.map(|[w, h]| [w.min(MAX_WINDOW), h.min(MAX_WINDOW)]);
        if winfiles_layout::grid(&self.print, &[]).is_err() {
            self.print = PrintSettings::default();
        }
        self.print.title.clear();
        self.calibrations = std::mem::take(&mut self.calibrations)
            .into_iter()
            .map(|(name, c)| (name, c.sanitized()))
            .filter(|(name, c)| !name.is_empty() && !c.is_identity())
            .take(MAX_CALIBRATIONS)
            .collect();
        self.presets = crate::presets::sanitized(std::mem::take(&mut self.presets));
        self
    }
}

/// Legge le impostazioni da `file`.
pub fn load_from(file: &Path) -> Prefs {
    let ok = std::fs::metadata(file).is_ok_and(|m| m.len() <= MAX_BYTES);
    if !ok {
        return Prefs::default();
    }
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str::<Prefs>(&text).ok())
        .unwrap_or_default()
        .sanitized()
}

/// Scrive le impostazioni in `file`, passando da un file temporaneo per non lasciarne uno a metà.
pub fn save_to(file: &Path, prefs: &Prefs) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(prefs).map_err(std::io::Error::other)?;
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, file)
}

/// Le impostazioni dell'utente.
pub fn load() -> Prefs {
    store().map(|f| load_from(&f)).unwrap_or_default()
}

/// Salva le impostazioni dell'utente; un errore non deve disturbare chi lavora.
pub fn save(prefs: &Prefs) {
    if let Some(f) = store() {
        let _ = save_to(&f, prefs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_ui::docking::Action;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("winfiles-prefs-{}-{name}", std::process::id()))
            .join("impostazioni.json")
    }

    #[test]
    fn salva_e_rilegge_tutte_le_scelte() {
        let file = temp("tutto");
        let mut dock = crate::dock::default_layout();
        dock.apply(Action::Close {
            panel: PanelId::Notes,
        })
        .unwrap();
        let mut print = PrintSettings {
            columns: 6,
            row_gap: 1.5,
            ..PrintSettings::default()
        };
        print.margins.top = 15.0;
        let prefs = Prefs {
            dock: Some(dock),
            theme: ThemeChoice::Light,
            unit: Unit::Cm,
            print,
            printing: PrintChoices {
                printer: Some("Ufficio A3".into()),
                sides: Sides::LongEdge,
                gray: true,
                fit: true,
            },
            calibrations: BTreeMap::from([(
                "Ufficio A3".to_string(),
                Calibration {
                    scale_x: 1.004,
                    offset_y: -0.5,
                    ..Calibration::IDENTITY
                },
            )]),
            presets: vec![crate::presets::Preset {
                name: "Catalogo A3".into(),
                print: PrintSettings {
                    paper: winfiles_geom::Paper::A3,
                    ..PrintSettings::default()
                },
            }],
            window: Some([1400.0, 900.0]),
            maximized: true,
            hide_unsupported: true,
            keep_legacy: true,
        };
        save_to(&file, &prefs).unwrap();
        assert_eq!(load_from(&file), prefs);
        std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }

    #[test]
    fn un_file_rovinato_o_assente_da_i_predefiniti() {
        let file = temp("rovinato");
        assert_eq!(load_from(&file), Prefs::default());
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "non è json").unwrap();
        assert_eq!(load_from(&file), Prefs::default());
        std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }

    #[test]
    fn i_campi_mancanti_o_assurdi_tornano_predefiniti() {
        let file = temp("parziale");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        // Un file di una versione vecchia con solo il tema, una finestra assurda e pannelli senza
        // l'anteprima.
        std::fs::write(
            &file,
            r#"{"theme":"Dark","window":[10.0,1e30],
               "dock":{"root":{"Tabs":{"panels":["Notes"],"active":0}},"floating":[]}}"#,
        )
        .unwrap();
        let p = load_from(&file);
        assert_eq!(p.theme, ThemeChoice::Dark);
        assert_eq!(p.window, None);
        assert_eq!(p.dock, None);
        assert_eq!(p.print, PrintSettings::default());
        assert!(p.calibrations.is_empty());
        std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }

    #[test]
    fn le_correzioni_assurde_si_lasciano_cadere() {
        let file = temp("calibrazioni");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            r#"{"calibrations":{
                "Buona":{"scale_x":1.01,"scale_y":0.99,"offset_x":0.5,"offset_y":0.0},
                "Doppia":{"scale_x":2.0},
                "Nulla":{},
                "Parziale":{"offset_x":-1.5}}}"#,
        )
        .unwrap();
        let p = load_from(&file);
        let names: Vec<&str> = p.calibrations.keys().map(String::as_str).collect();
        assert_eq!(names, ["Buona", "Parziale"]);
        assert_eq!(p.calibrations["Parziale"].scale_x, 1.0);
        std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }

    #[test]
    fn il_titolo_resta_del_lavoro() {
        let print = PrintSettings {
            title: "Cliente Rossi".into(),
            ..PrintSettings::default()
        };
        let p = Prefs {
            print,
            ..Prefs::default()
        }
        .sanitized();
        assert!(p.print.title.is_empty());
    }

    #[test]
    fn fronte_e_retro_avanti_e_indietro() {
        for s in [Sides::One, Sides::LongEdge, Sides::ShortEdge] {
            assert_eq!(Sides::from_duplex(s.duplex()), s);
        }
    }
}
