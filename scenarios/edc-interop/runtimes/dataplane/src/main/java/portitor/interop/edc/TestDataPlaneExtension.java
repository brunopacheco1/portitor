// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

package portitor.interop.edc;

import jakarta.ws.rs.Consumes;
import jakarta.ws.rs.GET;
import jakarta.ws.rs.HeaderParam;
import jakarta.ws.rs.POST;
import jakarta.ws.rs.Path;
import jakarta.ws.rs.PathParam;
import jakarta.ws.rs.core.MediaType;
import jakarta.ws.rs.core.Response;
import org.eclipse.dataplane.Dataplane;
import org.eclipse.dataplane.domain.DataAddress;
import org.eclipse.dataplane.domain.Result;
import org.eclipse.dataplane.domain.dataflow.DataFlow;
import org.eclipse.dataplane.domain.registration.Authorization;
import org.eclipse.dataplane.domain.registration.AuthorizationProfile;
import org.eclipse.dataplane.port.DataPlaneRegistrationApiController;
import org.eclipse.dataplane.port.DataPlaneSignalingApiController;
import org.eclipse.edc.runtime.metamodel.annotation.Inject;
import org.eclipse.edc.runtime.metamodel.annotation.Setting;
import org.eclipse.edc.spi.monitor.Monitor;
import org.eclipse.edc.spi.system.ServiceExtension;
import org.eclipse.edc.spi.system.ServiceExtensionContext;
import org.eclipse.edc.web.spi.WebService;

import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.security.MessageDigest;
import java.time.Duration;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

import static java.nio.charset.StandardCharsets.UTF_8;

/**
 * Test only: the smallest EDC data plane for "HttpData-PULL" transfers,
 * and for receiving "HttpData-PUSH" transfers as consumer.
 *
 * <ul>
 * <li>As provider, a started data flow gets a random token and the data
 * address {@code <public url>/public/<flow id>}; a GET there with the
 * token returns the file at {@code interop.dataplane.source.url}.</li>
 * <li>As consumer of a pull, it keeps the data address the provider sent,
 * and {@code GET <public url>/consumer/<flow id>/data} fetches the data
 * with it, so the test can read what an EDC consumer received.</li>
 * <li>As consumer of a push, it names where the provider sends the data:
 * when the control plane prepares the flow, the data address
 * {@code <public url>/push/<flow id>} with endpoint type
 * {@value #HTTP_PUSH_ENDPOINT_TYPE} (the Data Plane Signaling HTTP
 * profile), a random token ({@code authorization}) and {@code authType}
 * {@code bearer}. EDC v1.0.0-RC8 sends this address, as the data plane
 * gave it, in its TransferRequestMessage. A POST there with the token is
 * kept, and {@code GET <public url>/consumer/<flow id>/pushed} returns
 * it, so the test can compare it with the provider's file.</li>
 * </ul>
 *
 * A push is taken at once. EDC acknowledges the provider's
 * TransferStartMessage before its state machine has signalled this data
 * plane, and refuses a TransferCompletionMessage until then (400): the
 * provider must wait until EDC reports STARTED (Portitor does, ADR 0027).
 *
 * Signaling with the control plane uses no authentication (profile type
 * "none"), see the control plane's NoneSignalingAuthorizationExtension.
 */
public class TestDataPlaneExtension implements ServiceExtension {

    private static final String HTTP_PULL = "HttpData-PULL";
    private static final String HTTP_PUSH = "HttpData-PUSH";
    /** The push endpoint type of the Data Plane Signaling HTTP transfer profile. */
    static final String HTTP_PUSH_ENDPOINT_TYPE = "https://w3id.org/dspace-sig/profile/http-push";
    private static final String AUTHORIZATION = "authorization";

    @Setting(key = "dataplane.id", description = "Id of this data plane, as registered at the control plane")
    private String dataplaneId;
    @Setting(key = "interop.dataplane.public.url", description = "Base URL of this runtime's web API as others reach it")
    private String publicUrl;
    @Setting(key = "interop.dataplane.source.url", description = "The file served to consumers as provider")
    private String sourceUrl;

    @Inject
    private WebService webService;
    @Inject
    private Monitor monitor;

    private final HttpClient http = HttpClient.newBuilder().connectTimeout(Duration.ofSeconds(5)).build();
    /** Provider side: flow id to the token that opens its data. */
    private final Map<String, String> tokens = new ConcurrentHashMap<>();
    /** Consumer side: flow id to the data address the provider sent. */
    private final Map<String, DataAddress> received = new ConcurrentHashMap<>();
    /** Consumer side of a push: flow id to the token the provider must present. */
    private final Map<String, String> pushTokens = new ConcurrentHashMap<>();
    /** Consumer side of a push: flow id to the data the provider pushed (kept after the flow ends). */
    private final Map<String, byte[]> pushed = new ConcurrentHashMap<>();

    @Override
    public String name() {
        return "Portitor interop test data plane";
    }

    @Override
    public void initialize(ServiceExtensionContext context) {
        var dataplane = Dataplane.newInstance()
                .id(dataplaneId)
                .endpoint(URI.create(publicUrl + "/v1/dataflows"))
                .registerAuthorization(new NoneAuthorization())
                .profile(HTTP_PULL)
                .profile(HTTP_PUSH)
                .onPrepare(this::prepare)
                .onStart(this::start)
                .onStarted(this::started)
                .onSuspend(Result::success)
                .onResume(Result::success)
                .onCompleted(this::forget)
                .onTerminate(this::forget)
                .build();
        webService.registerResource(new DataPlaneSignalingApiController(dataplane));
        webService.registerResource(new DataPlaneRegistrationApiController(dataplane));
        webService.registerResource(new PublicApi(this));
        webService.registerResource(new ConsumerApi(this));
        webService.registerResource(new PushApi(this));
    }

    /** Consumer: a pull needs nothing; a push gets the address the provider sends the data to. */
    private Result<DataFlow> prepare(DataFlow flow) {
        if (!HTTP_PUSH.equals(flow.getProfile())) {
            return Result.success(flow);
        }
        var token = UUID.randomUUID().toString();
        pushTokens.put(flow.getId(), token);
        flow.setDataAddress(new DataAddress(HTTP_PUSH_ENDPOINT_TYPE, publicUrl + "/push/" + flow.getId(), List.of(
                new DataAddress.EndpointProperty("string", AUTHORIZATION, token),
                new DataAddress.EndpointProperty("string", "authType", "bearer"))));
        monitor.info("data flow %s prepared for a push".formatted(flow.getId()));
        return Result.success(flow);
    }

    /** Provider: open the data to the holder of a new token. */
    private Result<DataFlow> start(DataFlow flow) {
        if (!HTTP_PULL.equals(flow.getProfile())) {
            return Result.failure(new UnsupportedOperationException("profile " + flow.getProfile()));
        }
        var token = UUID.randomUUID().toString();
        tokens.put(flow.getId(), token);
        var property = new DataAddress.EndpointProperty("string", AUTHORIZATION, token);
        flow.setDataAddress(new DataAddress("https://w3id.org/idsa/v4.1/HTTP",
                publicUrl + "/public/" + flow.getId(), List.of(property)));
        monitor.info("data flow %s started as provider".formatted(flow.getId()));
        return Result.success(flow);
    }

    /** Consumer: keep the provider's data address (pull), or let the push in (push). */
    private Result<DataFlow> started(DataFlow flow) {
        // a push's address here is our own, sent back by the control plane
        if (!HTTP_PUSH.equals(flow.getProfile()) && flow.getDataAddress() != null) {
            received.put(flow.getId(), flow.getDataAddress());
        }
        monitor.info("data flow %s started as consumer".formatted(flow.getId()));
        return Result.success(flow);
    }

    private Result<DataFlow> forget(DataFlow flow) {
        tokens.remove(flow.getId());
        received.remove(flow.getId());
        pushTokens.remove(flow.getId());
        return Result.success(flow);
    }

    /** "Bearer x" or "x" to "x". */
    static String withoutBearer(String header) {
        return header.regionMatches(true, 0, "Bearer ", 0, 7) ? header.substring(7) : header;
    }

    private static boolean same(String a, String b) {
        return MessageDigest.isEqual(a.getBytes(UTF_8), b.getBytes(UTF_8));
    }

    Response fetch(String url, Optional<String> token) {
        var request = HttpRequest.newBuilder(URI.create(url)).timeout(Duration.ofSeconds(10)).GET();
        token.ifPresent(t -> request.header("Authorization", "Bearer " + withoutBearer(t)));
        try {
            var response = http.send(request.build(), HttpResponse.BodyHandlers.ofByteArray());
            return Response.status(response.statusCode()).entity(response.body()).build();
        } catch (IOException e) {
            monitor.warning("fetching %s failed: %s".formatted(url, e.getMessage()));
            return Response.status(Response.Status.BAD_GATEWAY).build();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return Response.status(Response.Status.SERVICE_UNAVAILABLE).build();
        }
    }

    /** The provider's data endpoint. Jersey needs a static class here. */
    @Path("/public")
    public static class PublicApi {
        private final TestDataPlaneExtension dataPlane;

        PublicApi(TestDataPlaneExtension dataPlane) {
            this.dataPlane = dataPlane;
        }

        @GET
        @Path("/{flowId}")
        public Response data(@PathParam("flowId") String flowId, @HeaderParam("Authorization") String header) {
            var expected = dataPlane.tokens.get(flowId);
            if (expected == null || header == null || !same(withoutBearer(header), expected)) {
                return Response.status(Response.Status.UNAUTHORIZED).build();
            }
            return dataPlane.fetch(dataPlane.sourceUrl, Optional.empty());
        }
    }

    /** What the consumer received, for the test to check. */
    @Path("/consumer")
    public static class ConsumerApi {
        private final TestDataPlaneExtension dataPlane;

        ConsumerApi(TestDataPlaneExtension dataPlane) {
            this.dataPlane = dataPlane;
        }

        @GET
        @Path("/{flowId}/data")
        public Response data(@PathParam("flowId") String flowId) {
            var address = dataPlane.received.get(flowId);
            if (address == null) {
                return Response.status(Response.Status.NOT_FOUND).build();
            }
            var token = address.endpointProperties().stream()
                    .filter(p -> AUTHORIZATION.equals(p.name()) || "access_token".equals(p.name()))
                    .map(DataAddress.EndpointProperty::value)
                    .findFirst();
            return dataPlane.fetch(address.endpoint(), token);
        }

        @GET
        @Path("/{flowId}/pushed")
        public Response pushed(@PathParam("flowId") String flowId) {
            var data = dataPlane.pushed.get(flowId);
            if (data == null) {
                return Response.status(Response.Status.NOT_FOUND).build();
            }
            return Response.ok(data, MediaType.APPLICATION_OCTET_STREAM_TYPE).build();
        }
    }

    /** Where a provider pushes the data of a push flow. */
    @Path("/push")
    public static class PushApi {
        private final TestDataPlaneExtension dataPlane;

        PushApi(TestDataPlaneExtension dataPlane) {
            this.dataPlane = dataPlane;
        }

        @POST
        @Path("/{flowId}")
        @Consumes(MediaType.WILDCARD)
        public Response receive(@PathParam("flowId") String flowId, @HeaderParam("Authorization") String header,
                                byte[] body) {
            var expected = dataPlane.pushTokens.get(flowId);
            if (expected == null || header == null || !same(withoutBearer(header), expected)) {
                return Response.status(Response.Status.UNAUTHORIZED).build();
            }
            dataPlane.pushed.put(flowId, body == null ? new byte[0] : body);
            dataPlane.monitor.info("data flow %s: received %d bytes by push".formatted(flowId,
                    dataPlane.pushed.get(flowId).length));
            return Response.ok().build();
        }
    }

    /** Signaling without authentication, matching the control plane's "none" profile. */
    private static final class NoneAuthorization implements Authorization {
        @Override
        public String type() {
            return "none";
        }

        @Override
        public Result<String> authorizationHeader(AuthorizationProfile profile) {
            return Result.success("Bearer none");
        }

        @Override
        public Result<String> extractCallerId(String authorizationHeader) {
            return Result.success("controlplane");
        }
    }
}
