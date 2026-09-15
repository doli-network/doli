//! SK-M1 outcome-metric probe (not an assertion test).
//!
//! Runs the REAL public installer entry point, `updater::install_skills_from_tarball`,
//! under a sudo-shaped environment (`HOME` = a throwaway process home, `SUDO_USER` = the
//! invoking operator). It asserts nothing about WHERE the files land — the probe script
//! `.claude/scripts/sk-m1-probe.sh` observes that on disk afterwards. Keeping the verdict
//! out of the test is what lets the same probe run unchanged before and after the fix.
//!
//! Driven only by `.claude/scripts/sk-m1-probe.sh`; skips when its env is absent so a
//! plain `cargo test` never writes outside the target directory.

use std::io::Write;

fn tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let buf = Vec::new();
    let enc = flate2::write::GzEncoder::new(buf, flate2::Compression::fast());
    let mut builder = tar::Builder::new(enc);
    for (path, body) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder.append_data(&mut header, path, *body).unwrap();
    }
    let mut enc = builder.into_inner().unwrap();
    enc.flush().unwrap();
    enc.finish().unwrap()
}

#[test]
fn sk_m1_probe_install_under_sudo_shaped_env() {
    if std::env::var("SK_M1_PROBE").ok().as_deref() != Some("1") {
        println!("SK-M1-PROBE=skipped (set SK_M1_PROBE=1 via .claude/scripts/sk-m1-probe.sh)");
        return;
    }
    let tar = tarball(&[(
        "pkg/skills/sk-m1-probe/SKILL.md",
        b"sk-m1 probe marker" as &[u8],
    )]);
    match updater::install_skills_from_tarball(&tar) {
        Ok(count) => println!("SK-M1-INSTALLED={count}"),
        Err(e) => println!("SK-M1-INSTALL-ERROR={e}"),
    }
    println!(
        "SK-M1-ENV home={} sudo_user={}",
        std::env::var("HOME").unwrap_or_default(),
        std::env::var("SUDO_USER").unwrap_or_default()
    );
}
