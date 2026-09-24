//! Tier 6: the server's own MAGPIE, run for real.
//!
//! The server builds every wordmap, rack info table and leave-generation KLV
//! with a pinned MAGPIE, so the things worth checking here are the ones no
//! amount of Rust-side testing can reach: that a scratch directory laid out by
//! `magpie.rs` is one MAGPIE will actually run in, that the conversions this
//! server invokes exist under the names it uses, and that the builder versions
//! it records come out of the binary rather than out of a guess.
//!
//! **Opt-in, then fail loudly.** `#[ignore]` by default; when you ask for these
//! and MAGPIE is missing they fail rather than skip, because a green run must
//! never silently mean nothing was exercised. Point `MAGPIE_BIN` at a build
//! (`make magpie BUILD=portable_release` in a MAGPIE checkout); the default
//! assumes a sibling checkout.
//!
//!     MAGPIE_BIN=../MAGPIE/bin/magpie cargo test --test magpie_smoke -- --ignored
//!
//! Nothing here touches Postgres or the object store. What needs those is the
//! end-to-end suite (`scripts/e2e_magpie.py`).

use birdtest::jobs::racks::LetterDistribution;
use birdtest::magpie::{Magpie, ScratchData};

const TESTDIST: &[u8] = include_bytes!("../src/jobs/testdata/testdist.csv");

fn magpie() -> Magpie {
    let binary =
        std::env::var("MAGPIE_BIN").unwrap_or_else(|_| "../MAGPIE/bin/magpie".to_string());
    assert!(
        std::path::Path::new(&binary).is_file(),
        "no MAGPIE at {binary}. Build one (`make magpie BUILD=portable_release` in a MAGPIE \
         checkout) and set MAGPIE_BIN. These tests fail rather than skip on purpose: a green \
         run must not mean nothing was exercised."
    );
    Magpie::new(binary, 1)
}

async fn scratch_with_distribution() -> (ScratchData, LetterDistribution) {
    let distribution = LetterDistribution::parse(TESTDIST, "testdist").unwrap();
    let scratch = ScratchData::empty().await.unwrap();
    scratch
        .write("letterdistributions", &distribution.name, ".csv", &distribution.bytes)
        .await
        .unwrap();
    (scratch, distribution)
}

/// The server reads the builder versions out of the binary at startup and
/// records them beside every hash it publishes. If this shape ever changes,
/// this is what says so — rather than every job an admin creates failing at
/// once with a parse error.
#[tokio::test]
#[ignore]
async fn the_binary_reports_its_builders() {
    let builders = magpie().builders().await.expect("magpie builders");
    assert!(!builders.magpie_version.is_empty());
    assert!(!builders.build_target.is_empty());
    assert!(builders.wmp_builder_version >= 1);
    assert!(builders.rit_builder_version >= 1);
    assert!(builders.klv_builder_version >= 1);
    assert_eq!(builders.wmp(), format!("wmp-{}", builders.wmp_builder_version));
}

/// Generation 0's zeroed KLV: every leave worth exactly nothing, built from the
/// letter distribution alone.
///
/// This also exercises the scratch directory, which is the fiddly part. MAGPIE
/// resolves data through `./data` relative to its working directory and loads a
/// default board layout *before* it parses any argument, so a directory that is
/// merely correct-looking fails every command — including `builders`.
#[tokio::test]
#[ignore]
async fn a_zeroed_klv_is_built_from_the_distribution_alone() {
    let magpie = magpie();
    let (scratch, distribution) = scratch_with_distribution().await;
    magpie
        .create_zero_klv(&scratch, "gen0", &distribution.name)
        .await
        .expect("createdata klv");
    let klv = tokio::fs::read(scratch.lexicon_path("gen0", ".klv2")).await.unwrap();
    assert!(!klv.is_empty(), "MAGPIE reported no error but wrote nothing");
}

/// A generation's KLV from its aggregated full-rack results.
///
/// The server streams one `rack,count,equity_sum` row per full rack — 3.2
/// million for English — and MAGPIE turns them into leave values. The row count
/// is the part worth pinning: a rack the file omits would contribute a mean of
/// zero at full draw weight to every leave it contains, which is a real leave
/// value and indistinguishable from a measured one, so MAGPIE refuses a file
/// that does not cover every rack exactly once rather than valuing the gaps.
#[tokio::test]
#[ignore]
async fn a_generations_klv_is_built_from_its_rack_equities() {
    use birdtest::jobs::racks::RackIndex;

    let magpie = magpie();
    let (scratch, distribution) = scratch_with_distribution().await;
    let index = RackIndex::new(&distribution, 7).unwrap();

    let mut rows = String::new();
    for i in 0..index.total() {
        let rack = index.rack_at(i).unwrap();
        // A distinct value per rack, so a permutation would show up as a
        // different KLV rather than as the same one.
        rows.push_str(&format!("{rack},{},{:.10}\n", 10 + i, (i as f64 % 13.0) - 6.0));
    }
    let complete = scratch.lexicon_path("gen1", ".csv");
    tokio::fs::write(&complete, &rows).await.unwrap();

    magpie
        .convert(&scratch, "rackequity2klv", "gen1", &distribution.name)
        .await
        .expect("rackequity2klv");
    let klv = tokio::fs::read(scratch.lexicon_path("gen1", ".klv2")).await.unwrap();
    assert!(!klv.is_empty());

    // Drop the last rack and it is refused, rather than valued at zero.
    let short: String = rows.lines().take(index.total() as usize - 1).collect::<Vec<_>>().join("\n");
    tokio::fs::write(&complete, format!("{short}\n")).await.unwrap();
    let err = magpie
        .convert(&scratch, "rackequity2klv", "gen1", &distribution.name)
        .await
        .expect_err("a CSV missing a rack must be refused");
    assert!(
        err.message.contains("full racks"),
        "the refusal should name what is missing: {}",
        err.message
    );
}

/// A wordmap, and a rack info table named for its (lexicon, leaves) pair.
///
/// The pair naming is the point. `klvwmp2rit` used to load the KLV and the
/// wordmap under the output's name, which forced a table to borrow one of
/// theirs and left birdtest no way to say which pair a file was for — so CSW24
/// with its own leaves and CSW24 with Quackle's would both have been
/// `CSW24.rit`. This passes the three names separately.
///
/// Uses MAGPIE's own two-letter test lexicon: a table for a real distribution
/// is 1.9 GB whatever the lexicon, because it is sized by the rack space.
#[tokio::test]
#[ignore]
async fn a_rack_info_table_is_built_for_a_named_pair() {
    let magpie = magpie();
    let root = std::env::var("MAGPIE_ROOT").unwrap_or_else(|_| "../MAGPIE".to_string());
    let testdata = std::path::Path::new(&root).join("testdata");
    let kwg = tokio::fs::read(testdata.join("lexica/CSW21_ab.kwg"))
        .await
        .expect("MAGPIE's CSW21_ab.kwg; set MAGPIE_ROOT");
    let klv = tokio::fs::read(testdata.join("lexica/CSW21_ab.klv2")).await.unwrap();
    let ld = tokio::fs::read(testdata.join("letterdistributions/english_ab.csv")).await.unwrap();

    let scratch = ScratchData::empty().await.unwrap();
    scratch.write("letterdistributions", "english_ab", ".csv", &ld).await.unwrap();
    scratch.write("lexica", "CSW21_ab", ".kwg", &kwg).await.unwrap();
    scratch.write("lexica", "quackle", ".klv2", &klv).await.unwrap();

    magpie.convert(&scratch, "dawg2wordmap", "CSW21_ab", "english_ab").await.unwrap();
    assert!(scratch.lexicon_path("CSW21_ab", ".wmp").exists());

    magpie
        .convert_rack_info_table(&scratch, "CSW21_ab.quackle", "english_ab", "quackle", "CSW21_ab")
        .await
        .expect("klvwmp2rit with separate input names");
    assert!(
        scratch.lexicon_path("CSW21_ab.quackle", ".rit").exists(),
        "the table is named for its pair, not for either input"
    );
}

/// Two builds from the same inputs produce the same bytes.
///
/// This is the property the whole derived-file check rests on: the server
/// builds a reference copy and a worker builds its own, and they have to agree.
/// MAGPIE pins it from its side too (`magpie_test builderhash`); this pins it
/// through the path birdtest actually uses, which is a subprocess against a
/// scratch directory rather than an in-process call.
#[tokio::test]
#[ignore]
async fn a_wordmap_is_a_function_of_its_inputs() {
    let magpie = magpie();
    let root = std::env::var("MAGPIE_ROOT").unwrap_or_else(|_| "../MAGPIE".to_string());
    let testdata = std::path::Path::new(&root).join("testdata");
    let kwg = tokio::fs::read(testdata.join("lexica/CSW21_ab.kwg")).await.unwrap();
    let ld = tokio::fs::read(testdata.join("letterdistributions/english_ab.csv")).await.unwrap();

    let mut digests = Vec::new();
    for _ in 0..2 {
        let scratch = ScratchData::empty().await.unwrap();
        scratch.write("letterdistributions", "english_ab", ".csv", &ld).await.unwrap();
        scratch.write("lexica", "CSW21_ab", ".kwg", &kwg).await.unwrap();
        magpie.convert(&scratch, "dawg2wordmap", "CSW21_ab", "english_ab").await.unwrap();
        let bytes = tokio::fs::read(scratch.lexicon_path("CSW21_ab", ".wmp")).await.unwrap();
        digests.push(hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&bytes)));
    }
    assert_eq!(digests[0], digests[1], "two builds of one wordmap differ");
}
