package com.raisindb.newsfeed.service;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.raisindb.newsfeed.dto.AuthTokensResponse;
import com.raisindb.newsfeed.security.UserContext;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.http.MediaType;
import org.springframework.stereotype.Service;
import org.springframework.web.reactive.function.client.WebClient;
import org.springframework.web.reactive.function.client.WebClientResponseException;

import java.util.Base64;
import java.util.Map;
import java.util.Optional;

/**
 * Service for authentication with external auth API.
 */
@Service
public class AuthService {

    private static final Logger log = LoggerFactory.getLogger(AuthService.class);

    private final WebClient webClient;
    private final ObjectMapper objectMapper;
    private final String repoId;

    public AuthService(@Value("${auth.api-url}") String authApiUrl,
                       @Value("${auth.repo-id}") String repoId,
                       ObjectMapper objectMapper) {
        this.webClient = WebClient.builder()
                .baseUrl(authApiUrl)
                .build();
        this.objectMapper = objectMapper;
        this.repoId = repoId;
    }

    public Optional<AuthTokensResponse> register(String email, String password, String displayName) {
        try {
            Map<String, Object> body = Map.of(
                    "email", email,
                    "password", password,
                    "display_name", displayName != null ? displayName : ""
            );

            return webClient.post()
                    .uri("/auth/{repo}/register", repoId)
                    .contentType(MediaType.APPLICATION_JSON)
                    .bodyValue(body)
                    .retrieve()
                    .bodyToMono(AuthTokensResponse.class)
                    .blockOptional();
        } catch (WebClientResponseException e) {
            log.error("Registration failed: {} - {}", e.getStatusCode(), e.getResponseBodyAsString());
            return Optional.empty();
        } catch (Exception e) {
            log.error("Registration failed: {}", e.getMessage());
            return Optional.empty();
        }
    }

    public Optional<AuthTokensResponse> login(String email, String password, boolean rememberMe) {
        try {
            Map<String, Object> body = Map.of(
                    "email", email,
                    "password", password,
                    "remember_me", rememberMe
            );

            return webClient.post()
                    .uri("/auth/{repo}/login", repoId)
                    .contentType(MediaType.APPLICATION_JSON)
                    .bodyValue(body)
                    .retrieve()
                    .bodyToMono(AuthTokensResponse.class)
                    .blockOptional();
        } catch (WebClientResponseException e) {
            log.error("Login failed: {} - {}", e.getStatusCode(), e.getResponseBodyAsString());
            return Optional.empty();
        } catch (Exception e) {
            log.error("Login failed: {}", e.getMessage());
            return Optional.empty();
        }
    }

    public Optional<UserContext> getUserFromToken(String accessToken) {
        if (accessToken == null || accessToken.isEmpty()) {
            return Optional.empty();
        }

        try {
            // Parse JWT payload (base64 decode middle section)
            String[] parts = accessToken.split("\\.");
            if (parts.length != 3) {
                return Optional.empty();
            }

            String payload = new String(Base64.getUrlDecoder().decode(parts[1]));
            JsonNode claims = objectMapper.readTree(payload);

            UserContext userContext = new UserContext();
            userContext.setId(claims.has("sub") ? claims.get("sub").asText() : null);
            userContext.setEmail(claims.has("email") ? claims.get("email").asText() : null);
            userContext.setDisplayName(claims.has("display_name") ? claims.get("display_name").asText() : null);

            // Check expiration
            if (claims.has("exp")) {
                long exp = claims.get("exp").asLong();
                if (System.currentTimeMillis() / 1000 > exp) {
                    log.debug("Token expired");
                    return Optional.empty();
                }
            }

            return Optional.of(userContext);
        } catch (Exception e) {
            log.error("Failed to parse token: {}", e.getMessage());
            return Optional.empty();
        }
    }
}
