// musl as the shadowed layer. same flow as glibc, musl data

use super::{Category, TestContext, Libc, ShadowedHarness, Test};
use crate::fetch;

pub struct ShadowMusl;

impl Test for ShadowMusl {
    fn name(&self) -> &str {
        "namespace: musl"
    }

    fn category(&self) -> Category {
        Category::Namespace
    }

    fn libc(&self) -> Option<Libc> {
        Some(Libc::Musl)
    }

    fn run(&self, tcx: &mut TestContext) {
        let src = fetch::void_musl().expect("fetch void musl source");
        ShadowedHarness { source: src, libc_name: "musl", loader: "/lib/ld-musl-x86_64.so.1" }
            .run(tcx);
    }
}
