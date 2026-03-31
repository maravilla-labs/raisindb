package com.raisindb.newsfeed.security;

import com.raisindb.newsfeed.service.AuthService;
import jakarta.servlet.FilterChain;
import jakarta.servlet.ServletException;
import jakarta.servlet.http.Cookie;
import jakarta.servlet.http.HttpServletRequest;
import jakarta.servlet.http.HttpServletResponse;
import org.springframework.security.authentication.UsernamePasswordAuthenticationToken;
import org.springframework.security.core.context.SecurityContextHolder;
import org.springframework.stereotype.Component;
import org.springframework.web.filter.OncePerRequestFilter;

import java.io.IOException;
import java.util.Collections;

/**
 * JWT authentication filter that extracts token from cookies.
 */
@Component
public class JwtAuthenticationFilter extends OncePerRequestFilter {

    private final AuthService authService;

    public JwtAuthenticationFilter(AuthService authService) {
        this.authService = authService;
    }

    @Override
    protected void doFilterInternal(HttpServletRequest request,
                                    HttpServletResponse response,
                                    FilterChain chain) throws ServletException, IOException {
        String accessToken = extractToken(request);

        if (accessToken != null) {
            authService.getUserFromToken(accessToken).ifPresent(userContext -> {
                // Store in request for controllers
                request.setAttribute("userContext", userContext);
                request.setAttribute("accessToken", accessToken);

                // Create RaisinDbUserContext
                RaisinDbUserContext raisinDbUserContext = new RaisinDbUserContext(accessToken, userContext);
                request.setAttribute("raisinDbUserContext", raisinDbUserContext);

                // Set Spring Security context
                var auth = new UsernamePasswordAuthenticationToken(
                        raisinDbUserContext, null, Collections.emptyList());
                SecurityContextHolder.getContext().setAuthentication(auth);
            });
        } else {
            // Set empty context for unauthenticated users
            request.setAttribute("raisinDbUserContext", new RaisinDbUserContext());
        }

        chain.doFilter(request, response);
    }

    private String extractToken(HttpServletRequest request) {
        if (request.getCookies() != null) {
            for (Cookie cookie : request.getCookies()) {
                if ("access_token".equals(cookie.getName())) {
                    return cookie.getValue();
                }
            }
        }
        return null;
    }
}
