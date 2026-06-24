// glibc as the shadowed layer. same flow as musl, glibc data

use super::{Category, TestContext, Libc, ShadowedHarness, Test};
use crate::fetch;

pub struct ShadowGlibc;

impl Test for ShadowGlibc {
    fn name(&self) -> &str {
        "namespace: glibc"
    }

    fn category(&self) -> Category {
        Category::Namespace
    }

    fn libc(&self) -> Option<Libc> {
        Some(Libc::Glibc)
    }

    fn run(&self, tcx: &mut TestContext) {
        let src = fetch::void_glibc().expect("fetch void glibc source");
        ShadowedHarness { source: src, libc_name: "glibc", loader: "/lib64/ld-linux-x86-64.so.2" }
            .run(tcx);
    }
}
