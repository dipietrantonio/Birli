//! Errors that can occur in Birli

use thiserror::Error;

#[derive(Error, Debug)]
/// An enum of all the errors possible in Birli
pub enum BirliError {
    /// An error derived from `FitsError`.
    #[error("{source_file}:{source_line}\nInvalid flag filename template. Must contain \"%%\" (or \"%%%\") for MWAX")]
    InvalidFlagFilenameTemplate {
        /// The file where the error originated (usually `file!()`)
        source_file: &'static str,
        /// The line number where the error originated (usually `line!()`)
        source_line: u32,
        /// The filename templte
        filename_template: String,
    },
    /// Error when opening a fits file.
    #[error("{source_file}:{source_line}\nCouldn't open {fits_filename}: {fits_error}")]
    FitsOpen {
        /// The [`fitsio::errors::Error`]
        fits_error: fitsio::errors::Error,
        /// The filename of the fits file
        fits_filename: String,
        /// The file where the error originated (usually `file!()`)
        source_file: &'static str,
        /// The line number where the error originated (usually `line!()`)
        source_line: u32,
    },
    /// A generic error associated with the fitsio crate.
    #[error("{source_file}:{source_line}\n{fits_filename} HDU {hdu_num}: {fits_error}")]
    // TODO: address this
    #[allow(clippy::upper_case_acronyms)]
    FitsIO {
        /// The [`fitsio::errors::Error`]
        fits_error: fitsio::errors::Error,
        /// The filename of the fits file where the error occurred
        fits_filename: String,
        /// The hdu number in the fits file where the error occurred
        hdu_num: usize,
        /// The file where the error originated (usually `file!()`)
        source_file: &'static str,
        /// The line number where the error originated (usually `line!()`)
        source_line: u32,
    },

    #[error("{0}")]
    /// Error derived from [`mwalib::FitsError`]
    FitsError(#[from] mwalib::FitsError),

    #[error("{0}")]
    /// Error derived from [`fitsio::errors::Error`]
    FitsioError(#[from] fitsio::errors::Error),

    /// Error to describe some kind of inconsistent state within an mwaf file.
    #[error("Inconsistent mwaf file (file: {file}, expected: {expected}, found: {found})")]
    MwafInconsistent {
        /// The filename of the fits file where the error occurred
        file: String,
        /// The value that was expected
        expected: String,
        /// The unexpected value that was found
        found: String,
    },

    #[error("Invalid GPUBox ID {found}, expected on of {expected}")]
    /// Error for an unexpected gpubox ID
    InvalidGpuBox {
        /// The value that was expected
        expected: String,
        /// The unexpected value that was found
        found: String,
    },

    #[error("No common timesteps found. CorrelatorContext timestep info: {timestep_info}")]
    /// Error for when gpuboxes provided have no overlapping visibilities
    NoCommonTimesteps {
        /// display of mwalib::CorrelatorContext::gpubox_time_map
        timestep_info: String,
    },

    #[error("No timesteps were provided. CorrelatorContext timestep info: {timestep_info}")]
    /// Error for when gpuboxes provided have no overlapping visibilities
    NoProvidedTimesteps {
        /// display of mwalib::CorrelatorContext::gpubox_time_map
        timestep_info: String,
    },
}
