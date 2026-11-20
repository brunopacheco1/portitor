// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

// EDC Issuer Service (the IdentityHub project's issuer): its own DID
// document, the DCP issuer service (metadata, credential requests and
// their status), status lists, and an admin API (holders, attestation
// and credential definitions, revocation). In-memory stores. The
// "oauth2" variant, as for the IdentityHub: its Identity API and its
// admin API take the tokens scripts/edc-interop.sh signs with a
// test-only key (the same JWKS, identity-api-jwks in compose.yaml). Its
// keys and the holders' access tokens are in the Vault.

val edcVersion: String by project

dependencies {
    runtimeOnly("org.eclipse.edc:issuerservice-oauth2-bom:$edcVersion")
    runtimeOnly("org.eclipse.edc:vault-hashicorp:$edcVersion")
}
