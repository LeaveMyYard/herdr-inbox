use super::*;

#[test]
fn revisions_live_in_the_sign_bit_so_no_crc32_is_ever_a_revision() {
    // Python test_schema_migrations: revision_stamp(1) == -2147483647
    assert_eq!(stamped_revision(-2_147_483_647), Some(1));
    assert_eq!(
        stamped_revision(REVISION_BASE + MIGRATION_REVISION),
        Some(MIGRATION_REVISION)
    );
    assert_eq!(stamped_revision(-1), Some((1 << 31) - 1));
    for legacy in [0, 1, 1_668_105_336, (1 << 31) - 1] {
        assert_eq!(stamped_revision(legacy), None, "{legacy}");
    }
}
