//! Le tabelle fisse: i tipi di file che WinFiles conosce, raggruppati come nella finestra delle
//! impostazioni, i programmi noti di una tipografia e i programmi consigliati per ogni tipo.

/// Il gruppo di un tipo nella finestra Programmi esterni.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Print,
    Images,
    Vector,
    Documents,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::Print, Group::Images, Group::Vector, Group::Documents];

    pub const fn label(self) -> &'static str {
        match self {
            Group::Print => "Stampa",
            Group::Images => "Immagini",
            Group::Vector => "Vettoriali e CAD",
            Group::Documents => "Documenti",
        }
    }
}

/// Un tipo di file: una o più estensioni che hanno gli stessi programmi.
#[derive(Debug, PartialEq, Eq)]
pub struct FileType {
    /// La chiave scritta in `impostazioni.json`.
    pub key: &'static str,
    /// Il nome mostrato.
    pub label: &'static str,
    pub group: Group,
    /// Le estensioni, minuscole e senza punto; la prima è quella che si chiede a Windows.
    pub exts: &'static [&'static str],
}

const fn t(key: &'static str, label: &'static str, group: Group, exts: &'static [&'static str]) -> FileType {
    FileType {
        key,
        label,
        group,
        exts,
    }
}

/// Tutti i tipi, nell'ordine della finestra.
pub const TYPES: &[FileType] = &[
    t("pdf", "PDF", Group::Print, &["pdf"]),
    t("ai", "AI", Group::Print, &["ai"]),
    t("eps", "EPS", Group::Print, &["eps", "epsf", "ps"]),
    t("jpg", "JPG", Group::Images, &["jpg", "jpeg", "jpe", "jfif"]),
    t("png", "PNG", Group::Images, &["png"]),
    t("tif", "TIFF", Group::Images, &["tif", "tiff"]),
    t("psd", "PSD", Group::Images, &["psd", "psb"]),
    t(
        "raw",
        "RAW",
        Group::Images,
        &[
            "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "dng", "pef", "orf", "rw2", "raf",
        ],
    ),
    t("heic", "HEIC", Group::Images, &["heic", "heif", "hif"]),
    t("webp", "WEBP", Group::Images, &["webp"]),
    t("bmp", "BMP", Group::Images, &["bmp"]),
    t("gif", "GIF", Group::Images, &["gif"]),
    t("tga", "TGA", Group::Images, &["tga"]),
    t("svg", "SVG", Group::Vector, &["svg", "svgz"]),
    t("dxf", "DXF", Group::Vector, &["dxf"]),
    t("dwg", "DWG", Group::Vector, &["dwg"]),
    t("plt", "PLT", Group::Vector, &["plt", "hpgl", "hpg"]),
    t("word", "Word", Group::Documents, &["docx", "doc", "rtf", "odt"]),
    t("excel", "Excel", Group::Documents, &["xlsx", "xls", "csv", "ods"]),
    t(
        "powerpoint",
        "PowerPoint",
        Group::Documents,
        &["pptx", "ppt", "odp"],
    ),
    t("indd", "InDesign", Group::Documents, &["indd", "idml"]),
];

/// Il tipo di un'estensione (senza punto, maiuscole o minuscole).
pub fn type_of_ext(ext: &str) -> Option<&'static FileType> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    TYPES.iter().find(|t| t.exts.contains(&ext.as_str()))
}

/// Il tipo per chiave.
pub fn type_by_key(key: &str) -> Option<&'static FileType> {
    TYPES.iter().find(|t| t.key == key)
}

/// Un programma che WinFiles conosce per nome: lo trova anche quando cambia versione o cartella.
#[derive(Debug, PartialEq, Eq)]
pub struct Family {
    /// La chiave scritta in `impostazioni.json` (stabile fra le versioni del programma).
    pub key: &'static str,
    /// Il nome completo.
    pub name: &'static str,
    /// Il nome breve sul pulsante.
    pub label: &'static str,
    /// I nomi dell'eseguibile (minuscoli); il primo è il preferito (Acrobat prima di Reader).
    pub exes: &'static [&'static str],
    /// Un pezzo dell'identità delle app dello Store (AppUserModelID).
    pub app_id: Option<&'static str>,
    /// L'inizio del nome della cartella dentro `Program Files\Adobe`, per i programmi Adobe.
    pub adobe_dir: Option<&'static str>,
    /// I tipi che sa aprire, anche se Windows non lo ha registrato per quei tipi.
    pub types: &'static [&'static str],
}

const IMAGES: &[&str] = &["jpg", "png", "tif", "heic", "webp", "bmp", "gif", "tga"];

/// I programmi noti, comuni in una tipografia.
pub const FAMILIES: &[Family] = &[
    Family {
        key: "acrobat",
        name: "Adobe Acrobat",
        label: "Acrobat",
        exes: &["acrobat.exe", "acrord32.exe"],
        app_id: None,
        adobe_dir: Some("Acrobat"),
        types: &["pdf", "ai"],
    },
    Family {
        key: "illustrator",
        name: "Adobe Illustrator",
        label: "Illustrator",
        exes: &["illustrator.exe"],
        app_id: None,
        adobe_dir: Some("Adobe Illustrator"),
        types: &["pdf", "ai", "eps", "svg", "dxf", "dwg"],
    },
    Family {
        key: "photoshop",
        name: "Adobe Photoshop",
        label: "Photoshop",
        exes: &["photoshop.exe"],
        app_id: None,
        adobe_dir: Some("Adobe Photoshop"),
        types: &[
            "jpg", "png", "tif", "psd", "raw", "heic", "webp", "bmp", "gif", "tga", "eps", "pdf",
        ],
    },
    Family {
        key: "indesign",
        name: "Adobe InDesign",
        label: "InDesign",
        exes: &["indesign.exe"],
        app_id: None,
        adobe_dir: Some("Adobe InDesign"),
        types: &["indd"],
    },
    Family {
        key: "lightroom",
        name: "Adobe Lightroom Classic",
        label: "Lightroom",
        exes: &["lightroom.exe"],
        app_id: None,
        adobe_dir: Some("Adobe Lightroom"),
        types: &["raw", "jpg", "tif"],
    },
    Family {
        key: "foto",
        name: "Foto di Windows",
        label: "Foto",
        exes: &[],
        app_id: Some("Microsoft.Windows.Photos"),
        adobe_dir: None,
        types: &["jpg", "png", "tif", "heic", "webp", "bmp", "gif", "raw"],
    },
    Family {
        key: "paint",
        name: "Paint",
        label: "Paint",
        exes: &["mspaint.exe"],
        app_id: Some("Microsoft.Paint"),
        adobe_dir: None,
        types: &["jpg", "png", "bmp", "gif", "tif"],
    },
    Family {
        key: "coreldraw",
        name: "CorelDRAW",
        label: "CorelDRAW",
        exes: &["coreldrw.exe"],
        app_id: None,
        adobe_dir: None,
        types: &["pdf", "ai", "eps", "svg", "dxf", "dwg", "plt"],
    },
    Family {
        key: "inkscape",
        name: "Inkscape",
        label: "Inkscape",
        exes: &["inkscape.exe"],
        app_id: None,
        adobe_dir: None,
        types: &["svg", "pdf", "ai", "eps"],
    },
    Family {
        key: "gimp",
        name: "GIMP",
        label: "GIMP",
        exes: &["gimp.exe", "gimp-3.0.exe", "gimp-2.10.exe"],
        app_id: None,
        adobe_dir: None,
        types: IMAGES,
    },
    Family {
        key: "autocad",
        name: "AutoCAD",
        label: "AutoCAD",
        exes: &["acad.exe"],
        app_id: None,
        adobe_dir: None,
        types: &["dxf", "dwg"],
    },
    Family {
        key: "trueview",
        name: "DWG TrueView",
        label: "TrueView",
        exes: &["dwgviewr.exe"],
        app_id: None,
        adobe_dir: None,
        types: &["dxf", "dwg"],
    },
    Family {
        key: "word",
        name: "Microsoft Word",
        label: "Word",
        exes: &["winword.exe"],
        app_id: None,
        adobe_dir: None,
        types: &["word"],
    },
    Family {
        key: "excel",
        name: "Microsoft Excel",
        label: "Excel",
        exes: &["excel.exe"],
        app_id: None,
        adobe_dir: None,
        types: &["excel"],
    },
    Family {
        key: "powerpoint",
        name: "Microsoft PowerPoint",
        label: "PowerPoint",
        exes: &["powerpnt.exe"],
        app_id: None,
        adobe_dir: None,
        types: &["powerpoint"],
    },
];

pub fn family(key: &str) -> Option<&'static Family> {
    FAMILIES.iter().find(|f| f.key == key)
}

/// Il programma noto con questo eseguibile (solo il nome del file, maiuscole o minuscole).
pub fn family_of_exe(file_name: &str) -> Option<&'static Family> {
    let name = file_name.to_ascii_lowercase();
    FAMILIES.iter().find(|f| f.exes.contains(&name.as_str()))
}

/// Il programma noto con questa identità di app dello Store.
pub fn family_of_app_id(app_id: &str) -> Option<&'static Family> {
    FAMILIES
        .iter()
        .find(|f| f.app_id.is_some_and(|id| app_id.starts_with(id)))
}

/// I programmi consigliati per ogni tipo: un elenco di posti, e per ogni posto i programmi in
/// ordine di preferenza (il primo installato prende il posto).
pub fn recommended(type_key: &str) -> &'static [&'static [&'static str]] {
    match type_key {
        "pdf" => &[&["acrobat"], &["illustrator"]],
        "ai" => &[&["illustrator"], &["acrobat"]],
        "eps" => &[&["illustrator"], &["photoshop"]],
        "jpg" | "png" | "tif" | "heic" | "webp" | "bmp" | "gif" | "tga" => &[&["foto"], &["photoshop"]],
        "psd" => &[&["photoshop"]],
        "raw" => &[&["photoshop"], &["lightroom"]],
        "svg" => &[&["illustrator"], &["inkscape"]],
        "dxf" => &[&["autocad", "trueview"], &["illustrator"]],
        "dwg" => &[&["autocad", "trueview"]],
        "plt" => &[&["coreldraw"]],
        "word" => &[&["word"]],
        "excel" => &[&["excel"]],
        "powerpoint" => &[&["powerpoint"]],
        "indd" => &[&["indesign"]],
        _ => &[],
    }
}
