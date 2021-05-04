use birli::{context_to_baseline_imgsets, cxx_aoflagger_new};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use glob::glob;
use mwalib::CorrelatorContext;
use std::env;
use std::path::Path;

fn get_context_ord_half_1196175296() -> CorrelatorContext {
    let test_dir = env::var("BIRLI_TEST_DIR").unwrap();
    let test_path = Path::new(&test_dir);
    let vis_path = test_path.join("1196175296_vis");
    let metafits_path = vis_path
        .join("1196175296.metafits")
        .to_str()
        .unwrap()
        .to_owned();
    let gpufits_glob = vis_path
        .join("1196175296_*gpubox*_00.fits")
        .to_str()
        .unwrap()
        .to_owned();
    let gpufits_files: Vec<String> = glob(gpufits_glob.as_str())
        .unwrap()
        .filter_map(Result::ok)
        .map(|path| path.to_str().unwrap().to_owned())
        .collect();
    CorrelatorContext::new(&metafits_path, &gpufits_files).unwrap()
}

fn bench_context_to_baseline_imgsets_ord_half_1196175296(c: &mut Criterion) {
    let aoflagger = unsafe { cxx_aoflagger_new() };
    let context = get_context_ord_half_1196175296();
    c.bench_function("context_to_baseline_imgsets - ord_half_1196175296", |b| {
        b.iter(|| context_to_baseline_imgsets(black_box(&aoflagger), black_box(&context)))
    });
}

criterion_group!(
    name = benches;
    config = Criterion::default().sample_size(10);
    targets =
        bench_context_to_baseline_imgsets_ord_half_1196175296,
);
criterion_main!(benches);
