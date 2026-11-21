// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

// EDC control plane with DCP (the "controlplane-dcp-bom" of the EDC
// Connector), secrets in HashiCorp Vault (shared with the IdentityHub, as
// in EDC's Minimum Viable Dataspace), and two small test extensions.

val edcVersion: String by project

dependencies {
    runtimeOnly("org.eclipse.edc:controlplane-dcp-bom:$edcVersion")
    runtimeOnly("org.eclipse.edc:vault-hashicorp:$edcVersion")
    // for NoneSignalingAuthorizationExtension
    implementation("org.eclipse.edc:data-plane-signaling-spi:$edcVersion")
    // for CredentialClaimPolicyExtension: the policy engine and the
    // credential model
    implementation("org.eclipse.edc:control-plane-spi:$edcVersion")
    implementation("org.eclipse.edc:core-spi:$edcVersion")
}
