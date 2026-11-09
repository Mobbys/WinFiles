//! I modelli di impaginazione: le scelte del pannello Impaginazione (foglio, vista, colonne,
//! margini, corpi dei testi, caselle) salvate con un nome, per ritrovarle con un clic su ogni
//! lavoro simile. Il titolo resta del lavoro. Stanno in `impostazioni.json`.

use serde::{Deserialize, Serialize};
use winfiles_model::PrintSettings;

/// Modelli al massimo.
pub const MAX_PRESETS: usize = 50;
/// Lunghezza massima del nome, in caratteri.
pub const MAX_NAME: usize = 60;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub print: PrintSettings,
}

/// Le impostazioni senza ciò che è del lavoro (il titolo).
fn shape(print: &PrintSettings) -> PrintSettings {
    PrintSettings {
        title: String::new(),
        ..print.clone()
    }
}

/// Il nome pulito: spazi tolti e al massimo [`MAX_NAME`] caratteri; None se resta vuoto.
pub fn clean_name(name: &str) -> Option<String> {
    let name: String = name.trim().chars().take(MAX_NAME).collect();
    let name = name.trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// Salva `print` come modello `name`: un modello con lo stesso nome (senza badare alle maiuscole)
/// si sostituisce. L'elenco resta in ordine alfabetico. Restituisce il nome usato.
pub fn save(presets: &mut Vec<Preset>, name: &str, print: &PrintSettings) -> Option<String> {
    let name = clean_name(name)?;
    let key = name.to_lowercase();
    presets.retain(|p| p.name.to_lowercase() != key);
    if presets.len() >= MAX_PRESETS {
        return None;
    }
    presets.push(Preset {
        name: name.clone(),
        print: shape(print),
    });
    presets.sort_by_key(|p| p.name.to_lowercase());
    Some(name)
}

/// Il modello uguale alle impostazioni di adesso, se c'è.
pub fn matching<'a>(presets: &'a [Preset], print: &PrintSettings) -> Option<&'a Preset> {
    let now = shape(print);
    presets.iter().find(|p| p.print == now)
}

/// Applica un modello al lavoro, tenendo il suo titolo.
pub fn apply(preset: &Preset, print: &mut PrintSettings) {
    let title = std::mem::take(&mut print.title);
    *print = PrintSettings {
        title,
        ..preset.print.clone()
    };
}

/// I modelli letti da file, senza quelli inutilizzabili: nome vuoto, doppione, impaginazione
/// impossibile.
pub fn sanitized(presets: Vec<Preset>) -> Vec<Preset> {
    let mut out = Vec::new();
    for p in presets {
        if winfiles_layout::grid(&p.print, &[]).is_err() {
            continue;
        }
        let _ = save(&mut out, &p.name, &p.print);
    }
    out
}

#[cfg(test)]
mod tests {
    use winfiles_geom::Paper;

    use super::*;

    fn a3() -> PrintSettings {
        PrintSettings {
            paper: Paper::A3,
            columns: 6,
            title: "Cliente Rossi".into(),
            ..PrintSettings::default()
        }
    }

    #[test]
    fn si_salva_e_si_ritrova_senza_il_titolo() {
        let mut presets = Vec::new();
        assert_eq!(
            save(&mut presets, "  Catalogo A3  ", &a3()).as_deref(),
            Some("Catalogo A3")
        );
        assert_eq!(presets[0].print.title, "");
        // Le stesse impostazioni con un altro titolo sono lo stesso modello.
        let mut other = a3();
        other.title = "Bianchi".into();
        assert_eq!(
            matching(&presets, &other).map(|p| p.name.as_str()),
            Some("Catalogo A3")
        );
        assert!(matching(&presets, &PrintSettings::default()).is_none());
        // Applicato, il titolo del lavoro resta.
        let mut job = PrintSettings {
            title: "Verdi".into(),
            ..PrintSettings::default()
        };
        apply(&presets[0], &mut job);
        assert_eq!(
            (job.paper, job.columns, job.title.as_str()),
            (Paper::A3, 6, "Verdi")
        );
    }

    #[test]
    fn lo_stesso_nome_sostituisce_e_l_ordine_e_alfabetico() {
        let mut presets = Vec::new();
        save(&mut presets, "Volantini", &PrintSettings::default());
        save(&mut presets, "catalogo", &PrintSettings::default());
        save(&mut presets, "Catalogo", &a3());
        let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Catalogo", "Volantini"]);
        assert_eq!(presets[0].print.paper, Paper::A3);
        assert_eq!(save(&mut presets, "   ", &a3()), None);
        let long = "x".repeat(200);
        assert_eq!(
            save(&mut presets, &long, &a3()).map(|n| n.chars().count()),
            Some(MAX_NAME)
        );
    }

    #[test]
    fn i_modelli_rovinati_si_lasciano_cadere() {
        let bad = PrintSettings {
            columns: 0,
            ..PrintSettings::default()
        };
        let list = vec![
            Preset {
                name: "Buono".into(),
                print: a3(),
            },
            Preset {
                name: "Rotto".into(),
                print: bad,
            },
            Preset {
                name: "buono".into(),
                print: PrintSettings::default(),
            },
            Preset {
                name: " ".into(),
                print: PrintSettings::default(),
            },
        ];
        let out = sanitized(list);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "buono");
    }
}
