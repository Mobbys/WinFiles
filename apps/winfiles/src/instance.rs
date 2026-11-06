//! Una finestra sola per "Apri in WinFiles" dal tasto destro di Esplora file.
//!
//! Esplora file avvia WinFiles una volta per ogni file selezionato. La prima copia tiene una
//! porta su 127.0.0.1 (diversa per ogni utente); le altre, avviate con `--aggiungi`, le mandano i
//! loro percorsi e chiudono subito, così tutti i file finiscono nella stessa finestra. Anche una
//! finestra già aperta riceve i file nello stesso modo.
//!
//! Il protocollo è di testo: una riga `WINFILES 2 <utente>`, una riga con la scheda che riceve i
//! file (`miniature`, `nesting` o `visualizza`), un percorso per riga, una riga vuota; chi riceve
//! risponde `OK`. Una finestra di una versione vecchia (`WINFILES 1`) rifiuta il messaggio, e chi
//! manda apre la sua finestra. Se qualcosa non va, chi manda apre la sua finestra come
//! sempre: nel caso peggiore si torna a una finestra per file, mai a file persi.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// La prima riga di ogni messaggio, prima del nome utente.
const HELLO: &str = "WINFILES 2";
/// Quanto aspetta chi manda i percorsi prima di arrendersi e aprire la sua finestra.
const WAIT: Duration = Duration::from_secs(3);
/// Il tetto ai percorsi di un messaggio: Esplora file ne manda uno per avvio.
const MAX_PATHS: usize = 10_000;
/// Il tetto alla lunghezza di una riga (un percorso lungo di Windows sta in 32 767 caratteri).
const MAX_LINE: usize = 140_000;

/// La scheda che riceve i file mandati da un'altra copia di WinFiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    /// "Apri in WinFiles": i file vanno nell'elenco delle miniature.
    #[default]
    Thumbnails,
    /// "Invia al nesting WinFiles".
    Nesting,
    /// Il doppio clic su un file di cui WinFiles è il programma predefinito.
    Viewer,
}

impl Target {
    fn word(self) -> &'static str {
        match self {
            Target::Thumbnails => "miniature",
            Target::Nesting => "nesting",
            Target::Viewer => "visualizza",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        [Target::Thumbnails, Target::Nesting, Target::Viewer]
            .into_iter()
            .find(|t| t.word() == word)
    }
}

/// Un invio: la scheda e i percorsi.
pub type Delivery = (Target, Vec<PathBuf>);

/// I percorsi arrivati da altre copie di WinFiles, per la finestra aperta.
pub struct Inbox {
    rx: Receiver<Delivery>,
    ctx: Arc<OnceLock<egui::Context>>,
}

impl Inbox {
    /// Collega la finestra: da qui in poi un arrivo la ridisegna e la porta davanti.
    pub fn attach(&self, ctx: &egui::Context) {
        let _ = self.ctx.set(ctx.clone());
    }

    /// Tutti gli invii arrivati dall'ultima volta, nell'ordine.
    pub fn take(&self) -> Vec<Delivery> {
        self.rx.try_iter().collect()
    }
}

/// Come parte questa copia di WinFiles.
pub enum Start {
    /// I percorsi sono andati alla finestra già aperta: questa copia chiude.
    Forwarded,
    /// Questa copia apre la sua finestra; con `Some` riceve anche i file delle altre.
    Window(Option<Inbox>),
}

/// Prende la porta dell'utente oppure, con `forward`, manda i percorsi a chi la tiene.
pub fn start(paths: &[PathBuf], forward: bool, target: Target) -> Start {
    start_on(port(&user()), &user(), paths, forward, target)
}

fn start_on(port: u16, user: &str, paths: &[PathBuf], forward: bool, target: Target) -> Start {
    match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))) {
        Ok(listener) => Start::Window(listen(listener, user.to_string())),
        Err(_) if forward && send(port, user, paths, target).is_ok() => Start::Forwarded,
        Err(_) => Start::Window(None),
    }
}

/// Ascolta in un thread a parte; senza thread la finestra va avanti da sola.
fn listen(listener: TcpListener, user: String) -> Option<Inbox> {
    let (tx, rx) = channel();
    let ctx: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
    let wake = ctx.clone();
    std::thread::Builder::new()
        .name("winfiles-istanza".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                if receive(stream, &user, &tx).is_ok()
                    && let Some(ctx) = wake.get()
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    ctx.request_repaint();
                }
            }
        })
        .ok()?;
    Some(Inbox { rx, ctx })
}

/// Legge un messaggio e lo passa alla finestra.
fn receive(stream: TcpStream, user: &str, tx: &Sender<Delivery>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(WAIT))?;
    stream.set_write_timeout(Some(WAIT))?;
    let mut reader = BufReader::new(&stream);
    let first = read_line(&mut reader)?;
    if first != format!("{HELLO} {user}") {
        return Err(invalid("messaggio di un altro programma o di un altro utente"));
    }
    let target = Target::from_word(&read_line(&mut reader)?).ok_or_else(|| invalid("scheda sconosciuta"))?;
    let mut paths = Vec::new();
    loop {
        let line = read_line(&mut reader)?;
        if line.is_empty() {
            break;
        }
        if paths.len() >= MAX_PATHS {
            return Err(invalid("troppi percorsi"));
        }
        paths.push(PathBuf::from(line));
    }
    (&stream).write_all(b"OK\n")?;
    tx.send((target, paths)).map_err(|_| invalid("finestra chiusa"))
}

/// Una riga senza il ritorno a capo, con un tetto alla lunghezza.
fn read_line<R: BufRead>(reader: &mut R) -> std::io::Result<String> {
    let mut line = String::new();
    let n = (&mut *reader).take(MAX_LINE as u64).read_line(&mut line)?;
    if n == 0 || !line.ends_with('\n') {
        return Err(invalid("messaggio interrotto"));
    }
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

/// Manda i percorsi a chi tiene la porta e aspetta il suo `OK`.
fn send(port: u16, user: &str, paths: &[PathBuf], target: Target) -> std::io::Result<()> {
    let mut message = format!("{HELLO} {user}\n{}\n", target.word());
    for p in paths {
        // Un percorso che non è testo valido non passerebbe intatto: meglio la propria finestra.
        let text = p.to_str().ok_or_else(|| invalid("percorso non valido"))?;
        if text.is_empty() || text.contains(['\n', '\r']) {
            return Err(invalid("percorso non valido"));
        }
        message.push_str(text);
        message.push('\n');
    }
    message.push('\n');
    let mut stream = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), WAIT)?;
    stream.set_read_timeout(Some(WAIT))?;
    stream.set_write_timeout(Some(WAIT))?;
    stream.write_all(message.as_bytes())?;
    let mut answer = String::new();
    BufReader::new(&stream).take(16).read_line(&mut answer)?;
    if answer.trim_end() == "OK" {
        Ok(())
    } else {
        Err(invalid("nessuna risposta"))
    }
}

/// Un percorso dalla riga di comando. Per la radice di un disco Esplora file scrive `"C:\"`, e le
/// regole delle virgolette di Windows lo leggono come `C:"`: si rimette la barra.
pub fn arg_path(arg: std::ffi::OsString) -> PathBuf {
    match arg.to_str() {
        Some(s) if s.len() > 1 && s.ends_with('"') => PathBuf::from(format!("{}\\", s.trim_end_matches('"'))),
        _ => arg.into(),
    }
}

fn invalid(why: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, why.to_string())
}

/// Il nome dell'utente di Windows (o di Linux, per le prove).
fn user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
        .replace(['\n', '\r'], "")
}

/// La porta dell'utente, fra 41000 e 60999: sempre la stessa per lo stesso nome (FNV-1a), così
/// due utenti dello stesso PC non si mandano i file a vicenda.
fn port(user: &str) -> u16 {
    let mut hash: u32 = 0x811c_9dc5;
    for b in format!("WinFiles {}", user.to_lowercase()).bytes() {
        hash = (hash ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    41_000 + u16::try_from(hash % 20_000).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una porta libera per la prova.
    fn free_port() -> u16 {
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn wait_for(inbox: &Inbox) -> Vec<Delivery> {
        for _ in 0..100 {
            let got = inbox.take();
            if !got.is_empty() {
                return got;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Vec::new()
    }

    #[test]
    fn la_seconda_copia_manda_i_file_alla_prima_e_chiude() {
        let port = free_port();
        let Start::Window(Some(inbox)) = start_on(port, "michel", &[], false, Target::Thumbnails) else {
            panic!("la prima copia deve aprire la finestra e ascoltare");
        };
        let files = vec![
            PathBuf::from("C:\\Lavori\\à è.pdf"),
            PathBuf::from("C:\\Lavori\\Cartella"),
        ];
        assert!(matches!(
            start_on(port, "michel", &files, true, Target::Thumbnails),
            Start::Forwarded
        ));
        assert_eq!(wait_for(&inbox), vec![(Target::Thumbnails, files)]);
    }

    #[test]
    fn senza_aggiungi_la_seconda_copia_apre_la_sua_finestra() {
        let port = free_port();
        let _first = start_on(port, "michel", &[], false, Target::Thumbnails);
        assert!(matches!(
            start_on(port, "michel", &[PathBuf::from("a.pdf")], false, Target::Thumbnails),
            Start::Window(None)
        ));
    }

    #[test]
    fn un_altro_utente_non_riceve_i_file() {
        let port = free_port();
        let Start::Window(Some(inbox)) = start_on(port, "michel", &[], false, Target::Thumbnails) else {
            panic!("la prima copia deve ascoltare");
        };
        assert!(matches!(
            start_on(port, "anna", &[PathBuf::from("a.pdf")], true, Target::Thumbnails),
            Start::Window(None)
        ));
        assert!(inbox.take().is_empty());
    }

    #[test]
    fn una_porta_presa_da_un_altro_programma_non_blocca_l_avvio() {
        let other = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = other.local_addr().unwrap().port();
        // Il programma accetta e chiude senza rispondere.
        std::thread::spawn(move || {
            for s in other.incoming().flatten() {
                drop(s);
            }
        });
        assert!(matches!(
            start_on(port, "michel", &[PathBuf::from("a.pdf")], true, Target::Thumbnails),
            Start::Window(None)
        ));
    }

    #[test]
    fn invia_al_nesting_arriva_con_la_sua_scheda() {
        let port = free_port();
        let Start::Window(Some(inbox)) = start_on(port, "michel", &[], false, Target::Thumbnails) else {
            panic!("la prima copia deve ascoltare");
        };
        let files = vec![PathBuf::from("C:\\Taglio\\sagoma.pdf")];
        assert!(matches!(start_on(port, "michel", &files, true, Target::Nesting), Start::Forwarded));
        assert!(matches!(start_on(port, "michel", &files, true, Target::Viewer), Start::Forwarded));
        let mut got = wait_for(&inbox);
        for _ in 0..100 {
            if got.len() >= 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
            got.extend(inbox.take());
        }
        assert_eq!(got, vec![(Target::Nesting, files.clone()), (Target::Viewer, files)]);
    }

    #[test]
    fn la_scheda_si_legge_come_si_scrive() {
        for t in [Target::Thumbnails, Target::Nesting, Target::Viewer] {
            assert_eq!(Target::from_word(t.word()), Some(t));
        }
        assert_eq!(Target::from_word("altro"), None);
    }

    #[test]
    fn la_radice_del_disco_ritrova_la_barra() {
        assert_eq!(arg_path("C:\"".into()), PathBuf::from("C:\\"));
        assert_eq!(arg_path("C:\\Lavori".into()), PathBuf::from("C:\\Lavori"));
        assert_eq!(arg_path("\"".into()), PathBuf::from("\""));
    }

    #[test]
    fn la_porta_dipende_dall_utente_e_resta_nell_intervallo() {
        assert_eq!(port("Michel"), port("michel"));
        assert_ne!(port("michel"), port("anna"));
        for u in ["", "a", "Administrator", "utente con spazi"] {
            assert!((41_000..61_000).contains(&port(u)));
        }
    }
}
