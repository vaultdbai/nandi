# Relay TLS test certificates

Test only. `ca.pem` is a made-up certificate authority. `server.pem` and `server-key.pem` are a certificate it signed for `localhost` and `127.0.0.1`, valid for 100 years. The CA's own key was thrown away after signing.

`relay/tests.rs` (`the_client_trusts_a_company_ca_for_wss`) serves `wss://` with them through the `nandi-relay` dev-dependency and checks two things: a client that trusts `ca.pem` connects, and one that doesn't is refused. The relay repo keeps its own copy for its server tests. Never use them for a real relay.
