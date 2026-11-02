//! Prova a mano: `cargo run --release -p winfiles-photo --example foto -- foto.nef uscita.png [anteprima]`
//! legge una foto RAW o un PSD, dice quanto ci ha messo e salva un PNG da guardare.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(input), Some(output)) = (args.first(), args.get(1)) else {
        eprintln!("uso: foto <foto RAW o PSD> <uscita.png> [anteprima]");
        return Ok(());
    };
    let quality = if args.get(2).is_some() {
        winfiles_photo::Quality::Preview
    } else {
        winfiles_photo::Quality::Full
    };
    let data = std::fs::read(input)?;
    let start = std::time::Instant::now();
    let photo = winfiles_photo::decode(&data, quality)?;
    println!(
        "{} × {} pixel, {:?}, {} bit, dpi {:?}, {} in {:.2} s",
        photo.image.width(),
        photo.image.height(),
        photo.mode,
        photo.bits,
        photo.dpi,
        photo.origin,
        start.elapsed().as_secs_f64()
    );
    photo.image.thumbnail(1600, 1600).to_rgb8().save(output)?;
    Ok(())
}
