//! Iterms related to the reading and writing of the FITS-based MWA Flag file format.
//!
//! # MWAF Format
//!
//! Similar to the GPUFits format, mwaf files come in a set for each observation, and there is one
//! .mwaf file per gpubox (coarse channel). This file contains a binary table of all the flags for
//! that coarse channel. There is one row for each timestep-baseline combination, and there is only
//! one column. Each cell in the table containsa binary vector of flags for each fine channel in
//! the coarse channel.

use crate::cxx_aoflagger::ffi::CxxFlagMask;
use crate::error::BirliError;
use clap::crate_version;
use cxx::UniquePtr;
use fitsio::tables::{ColumnDataDescription, ColumnDataType, ConcreteColumnDescription};
use fitsio::FitsFile;
use fitsio::{self, hdu::FitsHdu};
use mwalib::{
    CorrelatorContext, CorrelatorVersion, _get_required_fits_key, _open_hdu, fits_open_hdu,
    get_required_fits_key,
};
use regex::Regex;
use std::collections::BTreeMap;
use std::path::Path;

/// flag metadata which for a particular flag file in the set.
pub struct FlagFileHeaders {
    /// The `VERSION` key from the primary hdu
    // TODO: what is this actually used for?
    pub version: String,
    /// The `GPSTIME` key from the primary hdu
    pub obs_id: u32,
    /// The number of correlator fine channels per flag file, and the `NCHANS` key from the primary hdu.
    pub num_channels: usize,
    /// Total number of antennas (tiles) in the array, and the `NANTENNA` key from the primary hdu
    pub num_ants: usize,
    /// Number of timesteps in the observation, and the `NSCANS` key from the primary hdu
    pub num_timesteps: usize,
    /// The `NPOLS` key from the primary hdu
    pub num_pols: usize,
    /// The `GPUBOXNO` key from the primary hdu
    pub gpubox_id: usize,
    /// The `COTVER` key from the primary hdu
    pub cotter_version: String,
    /// The `COTVDATE` key from the primary hdu
    pub cotter_version_date: String,
    /// The width of each fine channel mask vector in bytes, or the `NAXIS1` key from the table hdu
    pub bytes_per_row: usize,
    /// The number of rows (timesteps × baselines), and the `NAXIS2` key from the table hdu.
    pub num_rows: usize,
    // TODO: is it useful to output aoflagger version and strategy?
}

impl FlagFileHeaders {
    /// Construct the [`FlagFileHeaders`] struct corresponding to the provided mwalib context and
    /// gpubox id.
    pub fn from_gpubox_context(gpubox_id: usize, context: &CorrelatorContext) -> Self {
        let num_fine_per_coarse = context.metafits_context.num_corr_fine_chans_per_coarse;
        FlagFileHeaders {
            version: "1.0".to_string(),
            obs_id: context.metafits_context.obs_id,
            num_channels: context.metafits_context.num_corr_fine_chans_per_coarse,
            num_ants: context.metafits_context.num_ants,
            num_timesteps: context.num_timesteps,
            num_pols: 1,
            gpubox_id,
            cotter_version: format!("Birli-{}", crate_version!()),
            // TODO: use something like https://github.com/rustyhorde/vergen
            cotter_version_date: "2021-04-14".to_string(),
            bytes_per_row: num_fine_per_coarse / 8 + usize::from(num_fine_per_coarse % 8 != 0),
            num_rows: context.num_timesteps * context.metafits_context.num_baselines,
        }
    }
}

/// A group of .mwaf Files for the same observation
pub struct FlagFileSet {
    gpubox_fptrs: BTreeMap<usize, FitsFile>,
}

// TODO: can this just take metafits context instead of a full context?
impl FlagFileSet {
    fn get_gpubox_filenames(
        corr_version: CorrelatorVersion,
        filename_template: &str,
        gpubox_ids: &Vec<usize>,
    ) -> Result<BTreeMap<usize, String>, BirliError> {
        let num_percents = match corr_version {
            CorrelatorVersion::Legacy | CorrelatorVersion::OldLegacy => 2,
            _ => 3,
        };
        let re_percents = Regex::new(format!("%{{{},}}+", num_percents).as_str()).unwrap();

        if !re_percents.is_match(filename_template) {
            return Err(BirliError::InvalidFlagFilenameTemplate {
                source_file: file!(),
                source_line: line!(),
                filename_template: String::from(filename_template),
            });
        }

        let gpubox_filenames: BTreeMap<usize, String> = gpubox_ids
            .iter()
            .map(|&gpubox_id| {
                (
                    gpubox_id,
                    re_percents
                        .replace(
                            filename_template,
                            format!("{:0width$}", gpubox_id, width = num_percents),
                        )
                        .to_string(),
                )
            })
            .collect();

        Ok(gpubox_filenames)
    }

    /// Create a new set of flag files
    ///
    /// `filename_template` is a template string which is expanded to the list of flag files in the
    /// set, by replacing the percent (`%`) characters with each coarse channel's zero-prefixed
    /// GPUBox ID. This is to maintain backwards compatibility with Cotter.
    ///
    /// For MWA Ord (legacy, pre-2021) correlator observations, the GPUBox ID is the two digit
    /// correlator channel host number corresponding to [`mwalib::CoarseChannel.corr_chan_number`]
    ///
    /// For MWAX correlator observations, the GPUBox ID is the three-digit received channel number
    /// corresponding to [`mwalib::CoarseChannel.rec_chan_number`].
    ///
    /// Be sure to specify the correct number of percent characters.
    ///
    /// # Errors
    ///
    /// Will fail if there are files already present at the paths specified in filename template.
    ///
    /// Will also fail if an invalid flag filename template is provided (wrong number of percents).
    pub fn new(
        context: &CorrelatorContext,
        filename_template: &str,
        // TODO: make this optional
        gpubox_ids: &Vec<usize>,
    ) -> Result<Self, BirliError> {
        let mut gpubox_fptrs: BTreeMap<usize, FitsFile> = BTreeMap::new();
        let gpubox_filenames =
            FlagFileSet::get_gpubox_filenames(context.corr_version, filename_template, gpubox_ids)?;
        for (gpubox_id, filename) in gpubox_filenames.into_iter() {
            match FitsFile::create(Path::new(&filename.to_string())).open() {
                Ok(fptr) => {
                    gpubox_fptrs.insert(gpubox_id, fptr);
                }
                Err(fits_error) => {
                    return Err(BirliError::FitsOpen {
                        fits_error,
                        fits_filename: filename.into(),
                        source_file: file!(),
                        source_line: line!(),
                    })
                }
            }
        }

        Ok(FlagFileSet { gpubox_fptrs })
    }

    /// Open an existing set of flag files, given an observation's context, the flag filename
    /// template, and a list of gpubox ids.
    pub fn open(
        context: &CorrelatorContext,
        filename_template: &str,
        gpubox_ids: &Vec<usize>,
    ) -> Result<Self, BirliError> {
        let mut gpubox_fptrs: BTreeMap<usize, FitsFile> = BTreeMap::new();
        let gpubox_filenames =
            FlagFileSet::get_gpubox_filenames(context.corr_version, filename_template, gpubox_ids)?;
        for (gpubox_id, filename) in gpubox_filenames.into_iter() {
            match FitsFile::open(Path::new(&filename.to_string())) {
                Ok(fptr) => {
                    gpubox_fptrs.insert(gpubox_id, fptr);
                }
                Err(fits_error) => {
                    return Err(BirliError::FitsOpen {
                        fits_error,
                        fits_filename: filename.into(),
                        source_file: file!(),
                        source_line: line!(),
                    })
                }
            }
        }

        Ok(FlagFileSet { gpubox_fptrs })
    }

    fn write_primary_hdu(
        fptr: &mut FitsFile,
        hdu: &FitsHdu,
        header: &FlagFileHeaders,
    ) -> Result<(), BirliError> {
        hdu.write_key(fptr, "VERSION", header.version.to_string())?;
        hdu.write_key(fptr, "GPSTIME", header.obs_id)?;
        hdu.write_key(fptr, "NCHANS", header.num_channels as u32)?;
        hdu.write_key(fptr, "NANTENNA", header.num_ants as u32)?;
        hdu.write_key(fptr, "NSCANS", header.num_timesteps as u32)?;
        hdu.write_key(fptr, "NPOLS", header.num_pols as u32)?;
        hdu.write_key(fptr, "GPUBOXNO", header.gpubox_id as u32)?;
        hdu.write_key(fptr, "COTVER", header.cotter_version.to_string())?;
        hdu.write_key(fptr, "COTVDATE", header.cotter_version_date.to_string())?;
        Ok(())
    }

    fn write_table_hdu(
        fptr: &mut FitsFile,
        hdu: &FitsHdu,
        header: &FlagFileHeaders,
    ) -> Result<(), BirliError> {
        hdu.write_key(fptr, "NAXIS1", header.bytes_per_row as u32)?;
        hdu.write_key(fptr, "NAXIS2", header.num_rows as u32)?;
        Ok(())
    }

    fn read_header(fptr: &mut FitsFile) -> Result<FlagFileHeaders, BirliError> {
        let hdu0 = fits_open_hdu!(fptr, 0)?;
        let hdu1 = fits_open_hdu!(fptr, 1)?;
        let header = FlagFileHeaders {
            version: get_required_fits_key!(fptr, &hdu0, "VERSION")?,
            obs_id: get_required_fits_key!(fptr, &hdu0, "GPSTIME")?,
            num_channels: get_required_fits_key!(fptr, &hdu0, "NCHANS")?,
            num_ants: get_required_fits_key!(fptr, &hdu0, "NANTENNA")?,
            num_timesteps: get_required_fits_key!(fptr, &hdu0, "NSCANS")?,
            num_pols: get_required_fits_key!(fptr, &hdu0, "NPOLS")?,
            gpubox_id: get_required_fits_key!(fptr, &hdu0, "GPUBOXNO")?,
            cotter_version: get_required_fits_key!(fptr, &hdu0, "COTVER")?,
            cotter_version_date: get_required_fits_key!(fptr, &hdu0, "COTVDATE")?,
            bytes_per_row: get_required_fits_key!(fptr, &hdu1, "NAXIS1")?,
            num_rows: get_required_fits_key!(fptr, &hdu1, "NAXIS2")?,
        };
        let baselines = header.num_ants * (header.num_ants + 1) / 2;
        if header.num_rows != header.num_timesteps * baselines {
            return Err(BirliError::MwafInconsistent {
                file: String::from(&fptr.filename),
                expected: "NSCANS * NANTENNA * (NANTENNA+1) / 2 = NAXIS2".to_string(),
                found: format!(
                    "{} * {} != {}",
                    header.num_timesteps, baselines, header.num_rows
                ),
            });
        }
        Ok(header)
    }

    // pub fn read_validated_header(
    //     context: &CorrelatorContext,
    //     fptr: &mut FitsFile,
    // ) -> Result<FlagFileHeaders, BirliError> {
    //     let headers = FlagFileSet::read_header(fptr)?;
    //     let header_baselines = headers.num_ants * (headers.num_ants + 1) / 2;
    //     if header_baselines != context.metafits_context.num_baselines {
    //         return Err(BirliError::MwafInconsistent {
    //             file: String::from(&fptr.filename),
    //             expected: "NANTENNA * (NANTENNA+1) / 2 = context.metafits_context.num_baselines"
    //                 .to_string(),
    //             found: format!(
    //                 "{} != {}",
    //                 header_baselines, context.metafits_context.num_baselines
    //             ),
    //         });
    //     };

    //     // TODO: check NSCANS?
    //     // if headers.num_timesteps > context.num_timesteps {
    //     //     return Err(BirliError::MwafInconsistent {
    //     //         file: String::from(&fptr.filename),
    //     //         expected: "NSCANS <= context.num_timesteps".to_string(),
    //     //         found: format!("{} > {}", headers.num_timesteps, context.num_timesteps),
    //     //     });
    //     // };

    //     if headers.bytes_per_row * 8 < context.metafits_context.num_corr_fine_chans_per_coarse {
    //         return Err(BirliError::MwafInconsistent {
    //             file: String::from(&fptr.filename),
    //             expected: "headers.bytes_per_row * 8 >= context.metafits_context.num_corr_fine_chans_per_coarse".to_string(),
    //             found: format!("{} < {}", headers.bytes_per_row, context.metafits_context.num_corr_fine_chans_per_coarse),
    //         });
    //     }

    //     Ok(headers)
    // }

    /// Write flags to disk, given an observation's [`mwalib::CorrelatorContext`], and a
    /// [`std::collections::BTreeMap`] mapping from each baseline in the observation to a
    /// [`CxxFlagMask`]
    ///
    /// The filename template should contain two or 3 percentage (`%`) characters which will be replaced
    /// by the gpubox id or channel number (depending on correlator type). See [`FlagFileSet::new`]
    ///
    pub fn write_baseline_flagmasks(
        &mut self,
        context: &CorrelatorContext,
        baseline_flagmasks: BTreeMap<usize, UniquePtr<CxxFlagMask>>,
    ) -> Result<(), BirliError> {
        let gpubox_chan_numbers: BTreeMap<usize, usize> = context
            .coarse_chans
            .iter()
            .map(|chan| (chan.gpubox_number, chan.corr_chan_number))
            .collect();

        for (&gpubox_id, fptr) in self.gpubox_fptrs.iter_mut() {
            // dbg!(&gpubox_id);
            let primary_hdu = fits_open_hdu!(fptr, 0)?;
            let header = FlagFileHeaders::from_gpubox_context(gpubox_id, context);
            let num_baselines = header.num_ants * (header.num_ants + 1) / 2;
            let num_fine_chans_per_coarse = header.num_channels;
            FlagFileSet::write_primary_hdu(fptr, &primary_hdu, &header)?;
            let chan_number = match gpubox_chan_numbers.get(&gpubox_id) {
                Some(chan_number) => chan_number,
                None => {
                    return Err(BirliError::InvalidGpuBox {
                        expected: format!("{:?}", gpubox_chan_numbers.keys()),
                        found: format!("{}", gpubox_id),
                    })
                }
            };
            let flags_colname = "FLAGS";
            let table_hdu = fptr.create_table(
                "EXTNAME".to_string(),
                &[ConcreteColumnDescription {
                    name: flags_colname.to_string(),
                    data_type: ColumnDataDescription::vector(
                        ColumnDataType::Bit,
                        num_fine_chans_per_coarse,
                    ),
                }],
            )?;
            FlagFileSet::write_table_hdu(fptr, &table_hdu, &header)?;

            let mut status = 0;
            for (baseline_idx, flagmask) in baseline_flagmasks.iter() {
                // TODO: Assert baseline_idx < num_baselines?
                let flag_buffer = flagmask.Buffer();
                let flag_stride = flagmask.HorizontalStride();
                let num_timesteps = flagmask.Width();
                for timestep_idx in 0..num_timesteps {
                    let row_idx = (timestep_idx * num_baselines) + baseline_idx;
                    // TODO: document this, it's not super clear
                    let mut cell: Vec<i8> = flag_buffer
                        .iter()
                        .skip(timestep_idx)
                        .step_by(flag_stride)
                        .skip(chan_number * num_fine_chans_per_coarse)
                        .take(num_fine_chans_per_coarse)
                        .map(|&flag| i8::from(flag))
                        .collect();
                    unsafe {
                        fitsio_sys::ffpclx(
                            fptr.as_raw(),
                            1,
                            1 + row_idx as i64,
                            1,
                            cell.len() as i64,
                            cell.as_mut_ptr(),
                            &mut status,
                        );
                    }
                    fitsio::errors::check_status(status).map_err(|e| BirliError::FitsIO {
                        fits_error: e,
                        fits_filename: String::from(&fptr.filename),
                        hdu_num: 1,
                        source_file: file!(),
                        source_line: line!(),
                    })?;
                }
            }
        }
        Ok(())
    }

    fn read_flags_raw(
        fptr: &mut FitsFile,
        flags_raw: &mut [i8],
        row_idx: Option<usize>,
    ) -> Result<(), BirliError> {
        let mut status = 0;
        let row_idx = row_idx.unwrap_or(0);
        unsafe {
            fitsio_sys::ffgcx(
                fptr.as_raw(),
                1,
                1 + row_idx as i64,
                1,
                flags_raw.len() as i64,
                flags_raw.as_mut_ptr(),
                &mut status,
            );
        }
        fitsio::errors::check_status(status).map_err(|e| BirliError::FitsIO {
            fits_error: e,
            fits_filename: String::from(&fptr.filename),
            hdu_num: 1,
            source_file: file!(),
            source_line: line!(),
        })?;

        Ok(())
    }

    /// Read raw flags and headers from disk, as a [`std::collections::BTreeMap`] mapping from each
    /// gpubox id to a tuple containing a [`FlagFileHeaders`] and the raw flags as a vector of bytes.
    pub fn read_chan_header_flags_raw(
        &mut self,
    ) -> Result<BTreeMap<usize, (FlagFileHeaders, Vec<i8>)>, BirliError> {
        let mut chan_header_flags_raw = BTreeMap::new();

        for (&gpubox_id, fptr) in self.gpubox_fptrs.iter_mut() {
            let header = FlagFileSet::read_header(fptr)?;
            let num_baselines = header.num_ants * (header.num_ants + 1) / 2;
            let mut flags_raw: Vec<i8> =
                vec![0; header.num_timesteps * num_baselines * header.num_channels];
            for timestep_idx in 0..header.num_timesteps {
                for baseline_idx in 0..num_baselines {
                    let row_idx = (timestep_idx * num_baselines) + baseline_idx;
                    let start_bit_idx = row_idx * header.num_channels;
                    let end_bit_idx = start_bit_idx + header.num_channels;
                    FlagFileSet::read_flags_raw(
                        fptr,
                        &mut flags_raw[start_bit_idx..end_bit_idx],
                        Some(row_idx),
                    )?;
                }
            }
            chan_header_flags_raw.insert(gpubox_id, (header, flags_raw));
        }

        Ok(chan_header_flags_raw)
    }
}

#[cfg(test)]
mod tests {
    use super::{FlagFileHeaders, FlagFileSet};
    use crate::cxx_aoflagger::ffi::{cxx_aoflagger_new, CxxFlagMask};
    use crate::error::BirliError;
    use cxx::UniquePtr;
    use fitsio::FitsFile;
    use mwalib::{
        CorrelatorContext, _get_optional_fits_key, _open_hdu, fits_open_hdu, get_optional_fits_key,
    };
    use std::collections::BTreeMap;
    use std::fs::File;
    use std::path::Path;
    use tempfile::tempdir;

    // TODO: deduplicate this from lib.rs
    fn get_mwax_context() -> CorrelatorContext {
        let metafits_path = "tests/data/1297526432_mwax/1297526432.metafits";
        let gpufits_paths = vec![
            "tests/data/1297526432_mwax/1297526432_20210216160014_ch117_000.fits",
            "tests/data/1297526432_mwax/1297526432_20210216160014_ch117_001.fits",
            "tests/data/1297526432_mwax/1297526432_20210216160014_ch118_000.fits",
            "tests/data/1297526432_mwax/1297526432_20210216160014_ch118_001.fits",
        ];
        CorrelatorContext::new(&metafits_path, &gpufits_paths).unwrap()
    }

    fn get_mwa_ord_context() -> CorrelatorContext {
        let metafits_path = "tests/data/1196175296_mwa_ord/1196175296.metafits";
        let gpufits_paths = vec![
            "tests/data/1196175296_mwa_ord/1196175296_20171201145440_gpubox01_00.fits",
            "tests/data/1196175296_mwa_ord/1196175296_20171201145540_gpubox01_01.fits",
            "tests/data/1196175296_mwa_ord/1196175296_20171201145440_gpubox02_00.fits",
            "tests/data/1196175296_mwa_ord/1196175296_20171201145540_gpubox02_01.fits",
        ];
        CorrelatorContext::new(&metafits_path, &gpufits_paths).unwrap()
    }

    #[test]
    fn test_flagfileset_enforces_percents_in_filename_template() {
        let mwax_context = get_mwax_context();
        let mwax_gpubox_ids = mwax_context
            .coarse_chans
            .iter()
            .map(|chan| chan.gpubox_number)
            .collect();
        let mwa_ord_context = get_mwa_ord_context();
        let mwa_ord_gpubox_ids = mwa_ord_context
            .coarse_chans
            .iter()
            .map(|chan| chan.gpubox_number)
            .collect();

        macro_rules! test_percent_enforcement {
            ($context:expr, $template_suffix:expr, $gpubox_ids:expr, $expected:pat) => {
                let tmp_dir = tempdir().unwrap();
                assert!(matches!(
                    FlagFileSet::new(
                        $context,
                        tmp_dir.path().join($template_suffix).to_str().unwrap(),
                        $gpubox_ids
                    ),
                    $expected
                ))
            };
        }
        test_percent_enforcement!(
            &mwax_context,
            "mwax_no_percents.mwaf",
            &mwax_gpubox_ids,
            Err(BirliError::InvalidFlagFilenameTemplate { .. })
        );
        test_percent_enforcement!(
            &mwa_ord_context,
            "mwa_ord_no_percents.mwaf",
            &mwa_ord_gpubox_ids,
            Err(BirliError::InvalidFlagFilenameTemplate { .. })
        );
        test_percent_enforcement!(
            &mwax_context,
            "mwax_insufficient_percents_2_%%.mwaf",
            &mwax_gpubox_ids,
            Err(BirliError::InvalidFlagFilenameTemplate { .. })
        );
        test_percent_enforcement!(
            &mwa_ord_context,
            "mwa_ord_sufficient_percents_2_%%.mwaf",
            &mwa_ord_gpubox_ids,
            Ok(FlagFileSet { .. })
        );
        test_percent_enforcement!(
            &mwax_context,
            "mwax_sufficient_percents_3_%%%.mwaf",
            &mwax_gpubox_ids,
            Ok(FlagFileSet { .. })
        );
        test_percent_enforcement!(
            &mwa_ord_context,
            "mwa_ord_sufficient_percents_3_%%%.mwaf",
            &mwax_gpubox_ids,
            Ok(FlagFileSet { .. })
        );
    }

    #[test]
    fn test_flagfileset_fails_with_existing() {
        let context = get_mwax_context();
        let gpubox_ids: Vec<usize> = context
            .coarse_chans
            .iter()
            .map(|chan| chan.gpubox_number)
            .collect();

        let tmp_dir = tempdir().unwrap();
        let filename_template = tmp_dir.path().join("Flagfile%%%.mwaf");

        let ok_gpuboxes = gpubox_ids[..1].to_vec();
        let colliding_gpuboxes = gpubox_ids[1..].to_vec();

        for gpubox_id in colliding_gpuboxes.iter() {
            let colliding_filename = tmp_dir
                .path()
                .join(format!("Flagfile{:03}.mwaf", gpubox_id));
            File::create(colliding_filename.to_str().unwrap()).unwrap();
        }

        assert!(matches!(
            FlagFileSet::new(&context, filename_template.to_str().unwrap(), &ok_gpuboxes).unwrap(),
            FlagFileSet { .. }
        ));
        assert!(matches!(
            FlagFileSet::new(
                &context,
                filename_template.to_str().unwrap(),
                &colliding_gpuboxes
            )
            .err(),
            Some(BirliError::FitsOpen { .. })
        ));
    }

    #[test]
    fn test_read_headers() {
        let test_dir = Path::new("tests/data/1247842824_flags/");

        let context = CorrelatorContext::new(
            &test_dir.join("1247842824.metafits"),
            &[test_dir.join("1247842824_20190722150008_gpubox01_00.fits")],
        )
        .unwrap();

        let gpubox_ids: Vec<usize> = context
            .coarse_chans
            .iter()
            .map(|chan| chan.gpubox_number)
            .collect();

        let filename_template = &test_dir.join("Flagfile%%.mwaf");
        let mut flag_file_set =
            FlagFileSet::open(&context, filename_template.to_str().unwrap(), &gpubox_ids).unwrap();

        for (&gpubox_id, mut fptr) in flag_file_set.gpubox_fptrs.iter_mut() {
            let header = FlagFileSet::read_header(&mut fptr).unwrap();
            assert_eq!(header.obs_id, 1247842824);
            assert_eq!(header.num_channels, 128);
            assert_eq!(header.num_ants, 128);
            assert_eq!(header.num_timesteps, 120);
            assert_eq!(header.gpubox_id, gpubox_id);
            assert_eq!(header.cotter_version, "4.5");
            assert_eq!(header.cotter_version_date, "2020-08-10");
        }
    }

    #[test]
    fn test_write_primary_hdu() {
        let context = get_mwax_context();
        let gpubox_ids: Vec<usize> = context
            .coarse_chans
            .iter()
            .map(|chan| chan.gpubox_number)
            .collect();

        let tmp_dir = tempdir().unwrap();
        let mut gpubox_paths = BTreeMap::new();
        for &gpubox_id in gpubox_ids.iter() {
            gpubox_paths.insert(
                gpubox_id,
                tmp_dir
                    .path()
                    .join(format!("Flagfile{:03}.mwaf", gpubox_id)),
            );
        }

        {
            for (&gpubox_id, path) in gpubox_paths.iter() {
                let mut fptr = FitsFile::create(path).open().unwrap();
                let primary_hdu = fits_open_hdu!(&mut fptr, 0).unwrap();
                FlagFileSet::write_primary_hdu(
                    &mut fptr,
                    &primary_hdu,
                    &FlagFileHeaders::from_gpubox_context(gpubox_id, &context),
                )
                .unwrap();
            }
        }

        for (&gpubox_id, path) in gpubox_paths.iter() {
            let mut flag_fptr = FitsFile::open(path).unwrap();
            let hdu = flag_fptr.primary_hdu().unwrap();

            let gps_time: Option<i32> =
                get_optional_fits_key!(&mut flag_fptr, &hdu, "GPSTIME").unwrap();
            assert_eq!(gps_time.unwrap(), context.metafits_context.obs_id as i32);

            let num_chans: Option<i32> =
                get_optional_fits_key!(&mut flag_fptr, &hdu, "NCHANS").unwrap();
            assert_eq!(
                num_chans.unwrap(),
                context.metafits_context.num_corr_fine_chans_per_coarse as i32
            );

            let num_ants: Option<i32> =
                get_optional_fits_key!(&mut flag_fptr, &hdu, "NANTENNA").unwrap();
            assert_eq!(num_ants.unwrap(), context.metafits_context.num_ants as i32);

            let num_scans: Option<i32> =
                get_optional_fits_key!(&mut flag_fptr, &hdu, "NSCANS").unwrap();
            assert_eq!(num_scans.unwrap(), context.num_timesteps as i32);

            let gpubox_no: Option<i32> =
                get_optional_fits_key!(&mut flag_fptr, &hdu, "GPUBOXNO").unwrap();
            assert_eq!(gpubox_no.unwrap(), gpubox_id as i32);
        }
    }

    #[test]
    fn test_read_flags_raw() {
        let test_dir = Path::new("tests/data/1247842824_flags/");

        let context = CorrelatorContext::new(
            &test_dir.join("1247842824.metafits"),
            &[test_dir.join("1247842824_20190722150008_gpubox01_00.fits")],
        )
        .unwrap();

        let gpubox_ids: Vec<usize> = context
            .coarse_chans
            .iter()
            .map(|chan| chan.gpubox_number)
            .collect();

        let filename_template = &test_dir.join("Flagfile%%.mwaf");
        let mut flag_file_set =
            FlagFileSet::open(&context, filename_template.to_str().unwrap(), &gpubox_ids).unwrap();

        for (_, fptr) in flag_file_set.gpubox_fptrs.iter_mut() {
            let table_hdu = fptr.hdu(1).unwrap();
            dbg!(table_hdu);
        }
        let chan_flags_raw = flag_file_set.read_chan_header_flags_raw().unwrap();

        assert_eq!(chan_flags_raw.keys().len(), 1);
        let (chan1_header, chan1_flags_raw) = chan_flags_raw.get(&1).unwrap();
        assert!(chan1_flags_raw.len() > 0);

        let num_baselines = chan1_header.num_ants * (chan1_header.num_ants + 1) / 2;

        let tests = [
            (0, 0, 0, i8::from(false)),
            (0, 0, 1, i8::from(false)),
            (0, 1, 0, i8::from(false)),
            (0, 1, 1, i8::from(false)),
            (1, 0, 0, i8::from(true)),
            (1, 0, 1, i8::from(true)),
            (1, 1, 0, i8::from(true)),
            (1, 1, 1, i8::from(true)),
        ];
        for (timestep_idx, baseline_idx, fine_chan_idx, expected_flag) in tests.iter() {
            let row_idx = timestep_idx * num_baselines + baseline_idx;
            let offset = row_idx * chan1_header.bytes_per_row + fine_chan_idx;
            assert_eq!(
                &chan1_flags_raw[offset], expected_flag,
                "with timestep {}, baseline {}, fine_chan {}, expected {} at row_idx {}, offset {}",
                timestep_idx, baseline_idx, fine_chan_idx, expected_flag, row_idx, offset
            );
        }
    }

    #[test]
    fn test_write_baseline_flagmasks() {
        let context = get_mwax_context();
        let gpubox_ids: Vec<usize> = context
            .coarse_chans
            .iter()
            .map(|chan| chan.gpubox_number)
            .collect();

        let tmp_dir = tempdir().unwrap();
        let filename_template = tmp_dir.path().join("Flagfile%%%.mwaf");

        let mut i = 0;
        let num_fine_chans_per_coarse = context.metafits_context.num_corr_fine_chans_per_coarse;

        let height = context.num_coarse_chans * num_fine_chans_per_coarse;
        let width = context.num_timesteps;
        let mut baseline_flagmasks: BTreeMap<usize, UniquePtr<CxxFlagMask>> = BTreeMap::new();

        unsafe {
            let aoflagger = cxx_aoflagger_new();
            for (coarse_chan_idx, _) in context.coarse_chans.iter().enumerate() {
                for (timestep_idx, _) in context.timesteps.iter().enumerate() {
                    for baseline_idx in 0..context.metafits_context.num_baselines {
                        if !baseline_flagmasks.contains_key(&baseline_idx) {
                            baseline_flagmasks
                                .insert(baseline_idx, aoflagger.MakeFlagMask(width, height, false));
                        };
                        let flag_mask_ptr = baseline_flagmasks.get_mut(&baseline_idx).unwrap();
                        let flag_stride = flag_mask_ptr.HorizontalStride();
                        let flag_buf = flag_mask_ptr.pin_mut().BufferMut();
                        for fine_chan_idx in 0..num_fine_chans_per_coarse {
                            let flag_offset_y =
                                coarse_chan_idx * num_fine_chans_per_coarse + fine_chan_idx;
                            let flag_idx = flag_offset_y * flag_stride + timestep_idx;
                            assert!(flag_idx < flag_stride * height);
                            dbg!(flag_idx, fine_chan_idx, i, 1 << fine_chan_idx & i);
                            flag_buf[flag_idx] = 1 << fine_chan_idx & i != 0;
                        }
                        i = (i + 1) % (1 << num_fine_chans_per_coarse);
                    }
                }
            }
        }

        {
            let mut flag_file_set =
                FlagFileSet::new(&context, filename_template.to_str().unwrap(), &gpubox_ids)
                    .unwrap();
            flag_file_set
                .write_baseline_flagmasks(&context, baseline_flagmasks)
                .unwrap();
        }

        for &gpubox_id in gpubox_ids.iter() {
            let flag_path = tmp_dir
                .path()
                .join(format!("Flagfile{:03}.mwaf", gpubox_id));
            let mut flag_fptr = FitsFile::open(flag_path).unwrap();
            let table_hdu = flag_fptr.hdu(1).unwrap();
            dbg!(table_hdu);
        }

        let mut flag_file_set =
            FlagFileSet::open(&context, filename_template.to_str().unwrap(), &gpubox_ids).unwrap();

        let chan_header_flags_raw = flag_file_set.read_chan_header_flags_raw().unwrap();

        assert_eq!(chan_header_flags_raw.keys().len(), 2);
        let (chan1_header, chan1_flags_raw) = chan_header_flags_raw.get(&117).unwrap();
        dbg!(chan1_flags_raw);

        let num_baselines = chan1_header.num_ants * (chan1_header.num_ants + 1) / 2;
        assert_eq!(chan1_header.num_timesteps, context.num_timesteps);
        assert_eq!(num_baselines, context.metafits_context.num_baselines);
        assert_eq!(chan1_header.num_channels, num_fine_chans_per_coarse);
        assert_eq!(
            chan1_flags_raw.len(),
            chan1_header.num_timesteps * num_baselines * chan1_header.num_channels
        );

        let tests = [
            (0, 0, 0, i8::from(false)),
            (0, 0, 1, i8::from(false)),
            (0, 1, 0, i8::from(true)),
            (0, 1, 1, i8::from(false)),
            (0, 2, 0, i8::from(false)),
            (0, 2, 1, i8::from(true)),
            (1, 0, 0, i8::from(true)),
            (1, 0, 1, i8::from(true)),
            (1, 1, 0, i8::from(false)),
            (1, 1, 1, i8::from(false)),
            (1, 2, 0, i8::from(true)),
            (1, 2, 1, i8::from(false)),
        ];
        for (timestep_idx, baseline_idx, fine_chan_idx, expected_flag) in tests.iter() {
            let row_idx = timestep_idx * num_baselines + baseline_idx;
            let offset = row_idx * num_fine_chans_per_coarse + fine_chan_idx;
            assert_eq!(
                &chan1_flags_raw[offset], expected_flag,
                "with timestep {}, baseline {}, fine_chan {}, expected {} at row_idx {}, offset {}",
                timestep_idx, baseline_idx, fine_chan_idx, expected_flag, row_idx, offset
            );
        }
    }
}
