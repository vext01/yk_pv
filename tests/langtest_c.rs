use lang_tester::LangTester;
use regex::Regex;
use std::{
    env,
    fs::{self, read_to_string},
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;
use tests::{mk_compiler, EXTRA_LINK};
use ykbuild::{completion_wrapper::CompletionWrapper, ykllvm_bin};

const COMMENT: &str = "//";

fn run_suite(opt: &'static str) {
    println!("Running C tests with opt level {}...", opt);

    // Tests with the filename prefix `debug_` are only run in debug builds.
    #[cfg(cargo_profile = "release")]
    let filter: fn(&Path) -> bool = |p| {
        if let Some(ext) = p.extension() {
            ext == "c"
                && !p
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("debug_")
        } else {
            false
        }
    };
    #[cfg(cargo_profile = "debug")]
    let filter: fn(&Path) -> bool = |p| {
        if let Some(ext) = p.extension() {
            ext == "c"
        } else {
            false
        }
    };

    let tempdir = TempDir::new().unwrap();
    let tempdir_p = tempdir.path().to_owned();

    // Generate a `compile_commands.json` database for clangd.
    let ccg = CompletionWrapper::new(ykllvm_bin("clang"), "c_tests");
    for (k, v) in ccg.build_env() {
        env::set_var(k, v);
    }
    let wrapper_path = ccg.wrapper_path();

    LangTester::new()
        .test_dir("c")
        .test_file_filter(filter)
        .test_extract(move |p| {
            let altp = p.with_extension(format!("c.{}", opt.strip_prefix('-').unwrap()));
            let p = if altp.exists() { altp.as_path() } else { p };
            read_to_string(p)
                .unwrap()
                .lines()
                .skip_while(|l| !l.starts_with(COMMENT))
                .take_while(|l| l.starts_with(COMMENT))
                .map(|l| &l[COMMENT.len()..])
                .collect::<Vec<_>>()
                .join("\n")
        })
        .test_cmds(move |p| {
            let mut exe = PathBuf::new();
            exe.push(&tempdir_p);
            exe.push(p.file_stem().unwrap());

            // Decide if we have extra objects to link to the test.
            let key = p.file_name().unwrap().to_str().unwrap();
            let extra_objs = EXTRA_LINK
                .get(key)
                .unwrap_or(&Vec::new())
                .iter()
                .map(|l| l.generate_obj(&tempdir_p))
                .collect::<Vec<PathBuf>>();

            let mut compiler = mk_compiler(wrapper_path.as_path(), &exe, p, opt, &extra_objs, true);
            compiler.env("YK_COMPILER_PATH", ykllvm_bin("clang"));
            let runtime = Command::new(exe.clone());
            vec![("Compiler", compiler), ("Run-time", runtime)]
        })
        .fm_options(|_, _, fmb| {
            // Use `{{}}` to match non-literal strings in tests.
            // E.g. use `%{{var}}` to capture the name of a variable.
            let ptn_re = Regex::new(r"\{\{.+?\}\}").unwrap();
            let text_re = Regex::new(r"[a-zA-Z0-9\._]+").unwrap();
            fmb.name_matcher(ptn_re, text_re)
        })
        .run();
    ccg.generate();

    dbg!(tempdir.path());
    if let Ok(save_dir) = env::var("YKT_SAVE_BINS") {
        let mut save_dir = PathBuf::from(save_dir);
        save_dir.push(&format!("C{}", opt));
        if !save_dir.exists() {
            fs::create_dir_all(&save_dir).unwrap();
        }
        for bin in fs::read_dir(tempdir.path()).unwrap() {
            let bin = bin.unwrap();
            let mut dest = save_dir.to_owned();
            dest.push(bin.file_name());
            fs::copy(bin.path(), dest).unwrap();
        }
    }
}

fn main() {
    // For now we can only compile with -O0 since higher optimisation levels introduce machine code
    // we currently don't know how to deal with, e.g. temporaries which break stackmap
    // reconstruction. This isn't a huge problem as in the future we will keep two versions of the
    // interpreter around and only swap to -O0 when tracing and run on higher optimisation levels
    // otherwise.
    run_suite("-O0");
}
