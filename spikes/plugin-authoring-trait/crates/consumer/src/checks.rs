//! Tiny PASS/FAIL reporter, so a run is traceable line by line and the exit
//! code carries the verdict.

use std::fmt::Debug;
use std::process::ExitCode;

pub struct Checks {
    passed: usize,
    failed: usize,
}

impl Checks {
    pub fn new() -> Self {
        Checks { passed: 0, failed: 0 }
    }

    pub fn check(&mut self, name: &str, ok: bool) {
        if ok {
            self.passed += 1;
            println!("PASS {name}");
        } else {
            self.failed += 1;
            println!("FAIL {name}");
        }
    }

    pub fn eq<T: PartialEq + Debug>(&mut self, name: &str, got: T, want: T) {
        if got == want {
            self.passed += 1;
            println!("PASS {name}");
        } else {
            self.failed += 1;
            println!("FAIL {name}: got {got:?}, want {want:?}");
        }
    }

    pub fn finish(self) -> ExitCode {
        println!("SPIKE_TRAIT_CHECKS passed={} failed={}", self.passed, self.failed);
        if self.failed == 0 { ExitCode::SUCCESS } else { ExitCode::from(1) }
    }
}
