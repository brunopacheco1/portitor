// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

package portitor.interop.edc;

import org.eclipse.edc.connector.controlplane.contract.spi.policy.ContractNegotiationPolicyContext;
import org.eclipse.edc.connector.controlplane.contract.spi.policy.TransferProcessPolicyContext;
import org.eclipse.edc.iam.verifiablecredentials.spi.model.CredentialSubject;
import org.eclipse.edc.iam.verifiablecredentials.spi.model.VerifiableCredential;
import org.eclipse.edc.participant.spi.ParticipantAgentPolicyContext;
import org.eclipse.edc.policy.engine.spi.AtomicConstraintRuleFunction;
import org.eclipse.edc.policy.engine.spi.PolicyEngine;
import org.eclipse.edc.policy.engine.spi.RuleBindingRegistry;
import org.eclipse.edc.policy.model.Operator;
import org.eclipse.edc.policy.model.Permission;
import org.eclipse.edc.runtime.metamodel.annotation.Inject;
import org.eclipse.edc.spi.system.ServiceExtension;
import org.eclipse.edc.spi.system.ServiceExtensionContext;

import java.util.List;
import java.util.Objects;

/**
 * Test only: lets a policy of the EDC provider ask for one claim of one
 * credential type, the constraint
 * {@code InteropTestCredential.level eq <value>} (the left operand follows
 * EDC's convention {@code <CredentialType>.<claim>}, as Portitor's ADR 0019).
 * EDC has no such function of its own: a deployment writes one for each
 * credential it uses, as EDC's Minimum Viable Dataspace does. Without it,
 * EDC refuses to store a policy with this left operand ("not bound to any
 * scopes").
 *
 * <p>The constraint holds if the caller presented a credential of that
 * type with that claim value. Before this function runs, EDC has checked
 * the caller's presentation: signature, validity, revocation, and that the
 * issuer is a trusted one (edc.iam.trustedissuer.* in
 * config/edc-controlplane.properties).
 */
public class CredentialClaimPolicyExtension implements ServiceExtension {

    static final String CREDENTIAL_TYPE = "InteropTestCredential";
    static final String CLAIM = "level";
    static final String LEFT_OPERAND = CREDENTIAL_TYPE + "." + CLAIM;
    /** Where EDC keeps the credentials of a verified presentation (DcpDefaultServicesExtension). */
    static final String CREDENTIALS_CLAIM = "vc";

    @Inject
    private PolicyEngine policyEngine;

    @Inject
    private RuleBindingRegistry ruleBindingRegistry;

    @Override
    public String name() {
        return "Portitor interop test: the policy constraint " + LEFT_OPERAND;
    }

    @Override
    public void initialize(ServiceExtensionContext context) {
        // checked when a contract is negotiated, and again when a transfer is asked for
        ruleBindingRegistry.bind(LEFT_OPERAND, ContractNegotiationPolicyContext.NEGOTIATION_SCOPE);
        ruleBindingRegistry.bind(LEFT_OPERAND, TransferProcessPolicyContext.TRANSFER_SCOPE);
        policyEngine.registerFunction(ContractNegotiationPolicyContext.class, Permission.class, LEFT_OPERAND, new ClaimFunction<>());
        policyEngine.registerFunction(TransferProcessPolicyContext.class, Permission.class, LEFT_OPERAND, new ClaimFunction<>());
    }

    private static final class ClaimFunction<C extends ParticipantAgentPolicyContext> implements AtomicConstraintRuleFunction<Permission, C> {

        @Override
        public boolean evaluate(Operator operator, Object rightValue, Permission rule, C context) {
            if (operator != Operator.EQ) {
                context.reportProblem(LEFT_OPERAND + " supports only the operator eq");
                return false;
            }
            if (!(context.participantAgent().getClaims().get(CREDENTIALS_CLAIM) instanceof List<?> credentials)) {
                context.reportProblem("the caller presented no credentials");
                return false;
            }
            for (var entry : credentials) {
                if (entry instanceof VerifiableCredential credential && credential.getType().contains(CREDENTIAL_TYPE)) {
                    for (CredentialSubject subject : credential.getCredentialSubject()) {
                        if (Objects.equals(subject.getClaims().get(CLAIM), rightValue)) {
                            return true;
                        }
                    }
                }
            }
            context.reportProblem("the caller presented no " + CREDENTIAL_TYPE + " with " + CLAIM + " " + rightValue);
            return false;
        }
    }
}
