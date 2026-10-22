//! Pannelli agganciabili, come in VectorCraft: si trascinano per la linguetta, si raggruppano, si
//! staccano come finestre mobili e si richiudono. L'anteprima è il "documento" al centro e non si
//! può spostare né chiudere.

use craft_ui::docking::{DockStyle, Layout, Node, Permissions, Placement, Zone};
use craft_ui::layout::{SplitAxis, SplitSize};

use crate::theme::{Tokens, semibold};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum PanelId {
    Canvas,
    Files,
    Properties,
    Notes,
    Layout,
}

impl PanelId {
    /// I pannelli che si possono aprire e chiudere dal menu Finestra.
    pub const TOOLS: [PanelId; 4] = [
        PanelId::Files,
        PanelId::Properties,
        PanelId::Notes,
        PanelId::Layout,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            PanelId::Canvas => "Anteprima",
            PanelId::Files => "File",
            PanelId::Properties => "Proprietà",
            PanelId::Notes => "Note",
            PanelId::Layout => "Impaginazione",
        }
    }

    pub fn permissions(self) -> Permissions {
        match self {
            PanelId::Canvas => Permissions::PROTECTED,
            _ => Permissions::default(),
        }
    }

    /// Dove riaprire un pannello chiuso.
    pub fn reopen_at(self) -> Placement<PanelId> {
        match self {
            PanelId::Files => Placement::Split(Zone::Left),
            _ => Placement::Split(Zone::Right),
        }
    }
}

fn tabs(panels: &[PanelId]) -> Box<Node<PanelId>> {
    Box::new(Node::Tabs {
        panels: panels.to_vec(),
        active: 0,
    })
}

/// File a sinistra, anteprima al centro, Impaginazione (aperta) e Proprietà sopra le Note a destra.
pub fn default_layout() -> Layout<PanelId> {
    Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedFirst(290.0),
            first: tabs(&[PanelId::Files]),
            second: Box::new(Node::Split {
                axis: SplitAxis::Horizontal,
                size: SplitSize::FixedSecond(300.0),
                first: tabs(&[PanelId::Canvas]),
                second: Box::new(Node::Split {
                    axis: SplitAxis::Vertical,
                    size: SplitSize::Ratio(0.62),
                    first: tabs(&[PanelId::Layout, PanelId::Properties]),
                    second: tabs(&[PanelId::Notes]),
                }),
            }),
        }),
        floating: Vec::new(),
    }
}

/// Il gruppo di linguette che contiene `panel`: pannelli, indice di quello attivo e se è una
/// finestra mobile.
pub fn group_of(layout: &Layout<PanelId>, panel: PanelId) -> Option<(Vec<PanelId>, usize, bool)> {
    fn find(node: &Node<PanelId>, panel: PanelId) -> Option<(Vec<PanelId>, usize)> {
        match node {
            Node::Tabs { panels, active } => panels.contains(&panel).then(|| (panels.clone(), *active)),
            Node::Split { first, second, .. } => find(first, panel).or_else(|| find(second, panel)),
            _ => None,
        }
    }
    if let Some((panels, active)) = layout.root.as_ref().and_then(|r| find(r, panel)) {
        return Some((panels, active, false));
    }
    layout
        .floating
        .iter()
        .find(|f| f.panels.contains(&panel))
        .map(|f| (f.panels.clone(), f.active, true))
}

/// Lo stile delle linguette: titoli sopra le schede, senza riquadri, con le etichette dei
/// comandi in italiano. Il fondo delle schede lo disegna il contenuto (vedi `app::card`).
pub fn style(ui: &egui::Ui) -> DockStyle {
    let t = Tokens::get(ui.ctx());
    let mut style = DockStyle::from_ui(ui);
    style.tab_height = 34.0;
    style.gap = 10.0;
    style.background = egui::Color32::TRANSPARENT;
    style.tab_background = egui::Color32::TRANSPARENT;
    style.active_background = egui::Color32::TRANSPARENT;
    style.text = t.text_strong;
    style.inactive_text = t.text_dim;
    style.border = egui::Stroke::NONE;
    style.accent = t.accent;
    style.font = semibold(13.5);
    style.float_label = "Stacca il pannello".into();
    style.close_label = "Chiudi il pannello".into();
    style.move_label = "Raggruppa con".into();
    style.panels_label = "Pannelli".into();
    style.resize_label = "Ridimensiona i pannelli".into();
    style.resize_window_label = "Ridimensiona la finestra del pannello".into();
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_is_valid_and_complete() {
        let layout = default_layout();
        assert!(layout.validate().is_ok());
        for p in PanelId::TOOLS {
            assert!(layout.contains(&p));
        }
        assert!(layout.contains(&PanelId::Canvas));
    }
}
