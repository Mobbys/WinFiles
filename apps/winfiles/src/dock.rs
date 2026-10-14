//! Pannelli agganciabili, come in VectorCraft: si trascinano per la linguetta, si raggruppano, si
//! staccano come finestre mobili e si richiudono. L'anteprima è il "documento" al centro e non si
//! può spostare né chiudere.

use craft_ui::docking::{DockStyle, Layout, Node, Permissions, Placement, Zone};
use craft_ui::layout::{SplitAxis, SplitSize};

use crate::theme::{TOKENS, semibold};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

/// File a sinistra, anteprima al centro, Proprietà e Impaginazione sopra le Note a destra.
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
                    first: tabs(&[PanelId::Properties, PanelId::Layout]),
                    second: tabs(&[PanelId::Notes]),
                }),
            }),
        }),
        floating: Vec::new(),
    }
}

/// Lo stile delle linguette e dei bordi, con le etichette dei comandi in italiano.
pub fn style(ui: &egui::Ui) -> DockStyle {
    let t = TOKENS;
    let mut style = DockStyle::from_ui(ui);
    style.tab_height = 30.0;
    style.gap = 2.0;
    style.background = t.panel;
    style.tab_background = t.tab_strip;
    style.active_background = t.panel;
    style.text = t.text_strong;
    style.inactive_text = t.text_dim;
    style.border = egui::Stroke::new(1.0, t.border);
    style.accent = t.accent;
    style.font = semibold(12.0);
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
