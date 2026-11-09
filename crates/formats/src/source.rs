//! Letture a posizione su un file o su dati in memoria, sempre controllate.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::ProbeError;

/// Una sorgente di byte con letture a posizione.
pub trait Source {
    /// Lunghezza totale in byte.
    fn len(&self) -> u64;

    /// Riempie `buf` leggendo da `offset`. Se il file finisce prima è danneggiato.
    fn read_exact_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), ProbeError>;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Fino a `max` byte dall'inizio (meno se il file è più corto).
    fn read_prefix(&mut self, max: usize) -> Result<Vec<u8>, ProbeError> {
        self.read_up_to(0, max)
    }

    /// Fino a `max` byte da `offset` (meno se il file è più corto).
    fn read_up_to(&mut self, offset: u64, max: usize) -> Result<Vec<u8>, ProbeError> {
        let left = self.len().saturating_sub(offset);
        let n = usize::try_from(left).unwrap_or(usize::MAX).min(max);
        let mut buf = vec![0; n];
        self.read_exact_at(offset, &mut buf)?;
        Ok(buf)
    }

    /// L'intero contenuto, se non supera `max` byte.
    fn read_all(&mut self, max: u64) -> Result<Vec<u8>, ProbeError> {
        let len = self.len();
        if len > max {
            return Err(ProbeError::TooLarge(len));
        }
        let n = usize::try_from(len).map_err(|_| ProbeError::TooLarge(len))?;
        self.read_up_to(0, n)
    }
}

/// Un file su disco.
pub(crate) struct FileSource {
    file: File,
    len: u64,
}

impl FileSource {
    pub(crate) fn open(path: &Path) -> Result<Self, ProbeError> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self { file, len })
    }
}

impl Source for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_exact_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), ProbeError> {
        let end = offset
            .checked_add(buf.len() as u64)
            .ok_or(ProbeError::Damaged("posizione fuori dal file"))?;
        if end > self.len {
            return Err(ProbeError::Damaged("il file finisce prima del previsto"));
        }
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(buf)?;
        Ok(())
    }
}

/// Dati già in memoria.
impl Source for &[u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }

    fn read_exact_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), ProbeError> {
        let start = usize::try_from(offset).map_err(|_| ProbeError::Damaged("posizione fuori dal file"))?;
        let end = start
            .checked_add(buf.len())
            .ok_or(ProbeError::Damaged("posizione fuori dal file"))?;
        let bytes = self
            .get(start..end)
            .ok_or(ProbeError::Damaged("il file finisce prima del previsto"))?;
        buf.copy_from_slice(bytes);
        Ok(())
    }
}

/// Letture di interi da un buffer già letto, senza mai uscire dai limiti.
pub(crate) fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        b.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}
pub(crate) fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        b.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}
pub(crate) fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}
pub(crate) fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}
