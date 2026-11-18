# Security Policy

Nandi's job is to keep AI agents in a locked box, so we treat any way out of that box as a serious bug.

## Report a Vulnerability

Please report it privately, not in a public issue:

- **GitHub:** [Report a vulnerability](https://github.com/vaultdbai/nandi/security/advisories/new) (a private advisory only the maintainers see), or
- **Email:** hello@vaultdb.ai, with "Security" in the subject.

Include the Nandi version (Settings or the installer name), your OS, and the steps to reproduce. We credit you in the release notes unless you'd rather we didn't.

### Response Timeline

- **Initial Response**: Within 48 hours
- **Assessment**: Within 7 days
- **Fix**: Timeline depends on severity
- **Disclosure**: After fix is available

## In Scope

What Nandi promises, and so what we most want to hear about:

| Area          | A bug looks like                                                                                         |
| ------------- | -------------------------------------------------------------------------------------------------------- |
| The container | An agent reads a host file outside its project folder and shared folders, or escapes the container       |
| Credentials   | A model key, a sign-in, a keyring secret or a host credential file reaches a container                   |
| The network   | An agent reaches a site not on its allow list, other than its model provider and the search engine       |
| Approvals     | A tool that requires approval runs without one, or an approval applies to a different action than shown  |
| The record    | A run or a tool call is missing from the record, or an audit log line changes without breaking the chain |
| The relay     | The relay, or anyone on it, reads a request's or an answer's content, which is sealed end to end         |
| Signed packs  | An unsigned or tampered pack or agent installs as verified                                               |

## Out of Scope

- What you approved. An approved command does what it says, inside the container.
- A compromised host: malware with your user's rights can read what you can.
- Your model provider: prompts go to the provider you choose.
- Rate limits, missing security headers on vaultdb.ai, and reports from automated scanners with no working exploit.

## Supported Versions

The latest release on [Releases](https://github.com/vaultdbai/nandi/releases/latest). Fixes ship in a new release; the app updates itself.

## The App's Own Security Measures

This app uses Tauri's security model:

- **Permissions**: Minimal system permissions via `capabilities/`
- **IPC**: Type-safe commands via tauri-specta
- **File Access**: Scoped to app directories by default
- **CSP**: Configured in `index.html`

## For Developers

### File Operations

```rust
// ✅ Validate paths - prevent traversal attacks
if filename.contains("..") {
    return Err("Invalid filename".into());
}

// ❌ Never trust raw user input for paths
std::fs::write(user_input, data)
```

### Secrets

- Never commit secrets to version control
- Use `.env.local` (gitignored) for local secrets
- Use GitHub Secrets for CI/CD

### Dependency Audits

```bash
pnpm audit
cargo audit
```

## Resources

- [Tauri Security Guide](https://tauri.app/security/)
- [OWASP Top 10](https://owasp.org/www-project-top-ten/)
