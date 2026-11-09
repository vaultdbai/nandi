# Task: Container isolation tested in CI

**From:** feature review against "trust, state, verification" (2026-11-08)
**Serves:** trust, verification
**Size:** S–M, one slice per check

The sandbox is the trust boundary, and its isolation is tested by hand only. Make each claim a test that runs in CI.

## What to do

- [ ] Run the container end-to-end tests (`runtime_e2e`) in a CI job with Docker (Linux runner).
- [ ] **SB-01:** a leak test for the harness container: no host credentials (`~/.claude/.credentials.json`, `~/.aws/credentials`, SSO caches, tokens in `pip.conf` or `.npmrc`) are readable inside it.
- [ ] **SB-13:** the internal network: a container can't reach the internet or the host except through the egress proxy.
- [ ] **SB-14, SB-15:** the egress container, and a CLI run going through it, refused for a site outside `network.allow`.
- [ ] **RT-05, RT-06:** inspect the started container: `cap_drop ALL`, `no-new-privileges`, limits, read-only mounts.
- [ ] Update each entry's "Automation" line in `docs/testing/` and drop the "manual" gap.

## Done when

Every item above fails CI if the isolation breaks. No new features.
