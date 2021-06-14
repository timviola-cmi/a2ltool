use clap::{App, Arg, ArgGroup, ArgMatches};

use dwarf::load_debuginfo;
use std::time::Instant;
use a2lfile::A2lObject;

mod ifdata;
mod dwarf;
mod update;
mod xcp;


struct A2lLogger {
    log: Vec<String>
}

impl a2lfile::Logger for A2lLogger {
    fn log_message(&mut self, msg: String) {
        self.log.push(msg);
    }
}

macro_rules! cond_print {
    ($verbose:ident, $now:ident, $formatexp:expr) => {
        if $verbose == 1 {
            println!("{}", $formatexp);
        } else if $verbose >= 2 {
            for line in $formatexp.split('\n') {
                if line == "" {
                    println!("");
                } else {
                    println!("[{:9.4}ms] {}", $now.elapsed().as_secs_f64() * 1000.0, line);
                }
            }
        }
    }
}


macro_rules! ext_println {
    ($verbose:ident, $now:ident, $formatexp:expr) => {
        if $verbose <= 1 {
            println!("{}", $formatexp);
        } else {
            for line in $formatexp.split('\n') {
                if line == "" {
                    println!("");
                } else {
                    println!("[{:9.4}ms] {}", $now.elapsed().as_secs_f64() * 1000.0, line);
                }
            }
        }
    }
}


fn main() {
    match core() {
        Ok(_) => {},
        Err(err) => println!("{}", err)
    }
}


// Implement all the operations supported by a2ltool
// They will always be performed in this order:
//  1) load input
//  2) additional consistency checks
//  3) load elf
//  4) merge at the module level
//  5) merge at the project level
//  6) merge includes (flatten)
//  7) update addresses
//  8) clean up ifdata
//  9) sort the file
// 10) output
fn core() -> Result<(), String> {
    let arg_matches = get_args();

    let strict = arg_matches.is_present("STRICT");
    let verbose = arg_matches.occurrences_of("VERBOSE");
    let debugprint = arg_matches.is_present("DEBUGPRINT");

    let now = Instant::now();
    cond_print!(verbose, now, format!("\na2ltool {} ({})\n", env!("VERGEN_BUILD_SEMVER"), env!("VERGEN_GIT_SHA_SHORT")));

    // load input
    let input_filename = arg_matches.value_of("INPUT").unwrap();
    let mut logger = A2lLogger { log: Vec::new() };
    let a2lresult = a2lfile::load(input_filename, Some(ifdata::A2MLVECTOR_TEXT.to_string()), &mut logger, strict);
    for msg in logger.log {
        cond_print!(verbose, now, format!("{}\n", msg));
    }
    let mut a2l_file = a2lresult?;
    cond_print!(verbose, now, format!("Input \"{}\" loaded", input_filename));
    if debugprint {
        // why not cond_print? in that case the output string must always be
        // formatted before cond_print can decide whether to print it. This can take longer than parsing the file.
        println!("================\n{:#?}\n================\n", a2l_file)
    }

    // show XCP settings
    if arg_matches.is_present("SHOW_XCP") {
        xcp::show_settings(&a2l_file, input_filename);
    }

    // additional consistency checks
    if arg_matches.is_present("CHECK") {
        cond_print!(verbose, now, format!("Performing consistency check for {}.", input_filename));
        let mut logger = A2lLogger { log: Vec::new() };
        a2l_file.check(&mut logger);
        if logger.log.len() == 0 {
            ext_println!(verbose, now, format!("Consistency check complete. No problems found."));
        } else {
            for  msg in &logger.log {
                ext_println!(verbose, now, format!("    {}", msg));
            }
            ext_println!(verbose, now, format!("Consistency check complete. {} problems reported.", logger.log.len()));
        }
    }

    // load elf
    let elf_info = if arg_matches.is_present("ELFFILE") {
        let elffile = arg_matches.value_of("ELFFILE").unwrap();
        let elf_info = load_debuginfo(elffile)?;
        cond_print!(verbose, now, format!("Variables and types loaded from \"{}\": {} variables available", elffile, elf_info.variables.len()));
        if debugprint {
            println!("================\n{:#?}\n================\n", elf_info);
        }
        Some(elf_info)
    } else {
        None
    };

    // merge at the module level
    if let Some(merge_modules) = arg_matches.values_of("MERGEMODULE") {
        for mergemodule in merge_modules {
            let mut merge_logger = A2lLogger { log: Vec::new() };
            let mut merge_a2l= a2lfile::load(mergemodule, None, &mut merge_logger, strict)?;
            
            a2l_file.merge_modules(&mut merge_a2l);
            cond_print!(verbose, now, format!("Merged A2l objects from \"{}\"\n", mergemodule));
        }
    }

    // merge at the project level
    if let Some(merge_projects) = arg_matches.values_of("MERGEPROJECT") {
        for mergeproject in merge_projects {
            let mut merge_logger = A2lLogger { log: Vec::new() };
            let merge_a2l= a2lfile::load(mergeproject, None, &mut merge_logger, strict)?;
    
            a2l_file.project.module.extend(merge_a2l.project.module);
            cond_print!(verbose, now, format!("Project level merge with \"{}\". There are now {} modules.\n", mergeproject, a2l_file.project.module.len()));
        }
    }

    // merge includes
    if arg_matches.is_present("MERGEINCLUDES") {
        a2l_file.merge_includes();
        cond_print!(verbose, now, format!("Include directives have been merged\n"));
    }

    // update addresses
    if arg_matches.is_present("UPDATE") || arg_matches.is_present("SAFE_UPDATE") {
        let preserve_unknown = arg_matches.is_present("SAFE_UPDATE");
        let summary = update::update_addresses(&mut a2l_file, &elf_info.as_ref().unwrap(), preserve_unknown);

        cond_print!(verbose, now, format!("Address update done\nSummary:"));
        cond_print!(verbose, now, format!("   characteristic: {} updated, {} not found", summary.characteristic_updated, summary.characteristic_not_updated));
        cond_print!(verbose, now, format!("   measurement: {} updated, {} not found", summary.measurement_updated, summary.measurement_not_updated));
        cond_print!(verbose, now, format!("   axis_pts: {} updated, {} not found", summary.axis_pts_updated, summary.axis_pts_not_updated));
        cond_print!(verbose, now, format!("   blob: {} updated, {} not found", summary.blob_updated, summary.blob_not_updated));
        cond_print!(verbose, now, format!("   instance: {} updated, {} not found", summary.instance_updated, summary.instance_not_updated));
    }

    // remove unknown IF_DATA
    if arg_matches.is_present("IFDATA_CLEANUP") {
        a2l_file.ifdata_cleanup();
        cond_print!(verbose, now, format!("Unknown ifdata removal is done"));
    }

    // sort all elements in the file
    if arg_matches.is_present("SORT") {
        a2l_file.sort();
        cond_print!(verbose, now, format!("All objects have been sorted"));
    }

    // output
    if arg_matches.is_present("OUTPUT") {
        a2l_file.sort_new_items();
        let out_filename = arg_matches.value_of("OUTPUT").unwrap();
        let banner = &*format!("a2ltool {} ({})", env!("VERGEN_GIT_SEMVER"), env!("VERGEN_GIT_SHA_SHORT"));
        a2l_file.write(out_filename, Some(banner))?;
        cond_print!(verbose, now, format!("Output written to \"{}\"", out_filename));
    }


    cond_print!(verbose, now, format!("\nRun complete. Have a nice day!\n\n"));

    Ok(())
}


// set up the entire command line handling.
// fortunately clap makes this painless
fn get_args<'a>() -> ArgMatches<'a> {
    App::new("a2ltool")
    .version(&*format!("{} ({})", env!("VERGEN_GIT_SEMVER"), env!("VERGEN_GIT_SHA_SHORT")))
    .about("Reads, writes and modifies A2L files")
    .arg(Arg::with_name("INPUT")
        .help("Input A2L file")
        .required(true)
        .index(1)
    )
    .arg(Arg::with_name("ELFFILE")
        .help("Elf file containing symbols and address information")
        .short("e")
        .long("elffile")
        .takes_value(true)
        .value_name("ELFFILE")
    )
    .arg(Arg::with_name("CHECK")
        .help("Perform additional consistency checks")
        .long("check")
        .takes_value(false)
        .multiple(false)
    )
    .arg(Arg::with_name("MERGEMODULE")
        .help("Merge another a2l file on the MODULE level.\nThe input file and the merge file must each contain exactly one MODULE.\nThe contents will be merged so that there is one merged MODULE in the output.")
        .short("m")
        .long("merge")
        .takes_value(true)
        .value_name("A2LFILE")
        .number_of_values(1)
        .multiple(true)
    )
    .arg(Arg::with_name("MERGEPROJECT")
        .help("Merge another a2l file on the PROJECT level.\nIf the input file contains m MODULES and the merge file contains n MODULES, then there will be m + n MODULEs in the output.")
        .short("p")
        .long("merge-project")
        .takes_value(true)
        .value_name("A2LFILE")
        .number_of_values(1)
        .multiple(true)
    )
    .arg(Arg::with_name("MERGEINCLUDES")
        .help("Merge the content of all included files. The output file will contain no /include commands.")
        .short("i")
        .long("merge-includes")
        .takes_value(false)
        .multiple(false)
    )
    .arg(Arg::with_name("UPDATE")
        .help("Update the addresses of all objects in the A2L file based on the elf file.\nObjects that cannot be found in the elf file will be deleted.\nThe arg --elffile must be present.")
        .short("u")
        .long("update")
        .takes_value(false)
        .multiple(false)
        .requires("ELFFILE")
    )
    .arg(Arg::with_name("SAFE_UPDATE")
        .help("Update the addresses of all objects in the A2L file based on the elf file.\nObjects that cannot be found in the elf file will be preserved; their adresses will be set to zero.\nThe arg --elffile must be present.")
        .long("update-preserve")
        .takes_value(false)
        .multiple(false)
        .requires("ELFFILE")
    )
    .arg(Arg::with_name("OUTPUT")
        .help("Write to the given output file. If this flag is not present, no output will be written.")
        .short("o")
        .long("output")
        .takes_value(true)
        .value_name("A2LFILE")
        .multiple(false)
    )
    .arg(Arg::with_name("STRICT")
        .help("Parse all input in strict mode. An error wil be reported if the file has any inconsistency.")
        .short("s")
        .long("strict")
        .takes_value(false)
        .multiple(false)
    )
    .arg(Arg::with_name("VERBOSE")
        .help("Display additional information")
        .short("v")
        .long("verbose")
        .takes_value(false)
        .multiple(true)
    )
    .arg(Arg::with_name("DEBUGPRINT")
        .help("Display internal data for debugging")
        .long("debug-print")
        .takes_value(false)
        .multiple(false)
    )
    .arg(Arg::with_name("SORT")
        .help("Sort all the elements in the file")
        .long("sort")
        .takes_value(false)
        .multiple(false)
    )
    .arg(Arg::with_name("IFDATA_CLEANUP")
        .help("Remove all IF_DATA blocks that cannot be parsed according to A2ML")
        .long("ifdata-cleanup")
        .takes_value(false)
        .multiple(false)
    )
    .arg(Arg::with_name("SHOW_XCP")
        .help("Display the XCP settings in the a2l file, if they exist")
        .long("show-xcp")
        .takes_value(false)
        .multiple(false)
    )
    .group(
        ArgGroup::with_name("UPDATE_GROUP")
            .args(&["UPDATE", "SAFE_UPDATE"])
            .multiple(false)
    )
    .get_matches()
}