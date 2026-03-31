package com.raisindb.newsfeed.controller;

import com.raisindb.newsfeed.dto.AuthTokensResponse;
import com.raisindb.newsfeed.dto.LoginDto;
import com.raisindb.newsfeed.dto.RegisterDto;
import com.raisindb.newsfeed.service.AuthService;
import jakarta.servlet.http.Cookie;
import jakarta.servlet.http.HttpServletResponse;
import jakarta.validation.Valid;
import org.springframework.stereotype.Controller;
import org.springframework.ui.Model;
import org.springframework.validation.BindingResult;
import org.springframework.web.bind.annotation.*;

import java.util.Optional;

/**
 * Controller for authentication operations.
 */
@Controller
@RequestMapping("/auth")
public class AuthController {

    private final AuthService authService;

    public AuthController(AuthService authService) {
        this.authService = authService;
    }

    @GetMapping("/login")
    public String loginForm(@RequestParam(required = false) String redirect, Model model) {
        LoginDto loginDto = new LoginDto();
        loginDto.setRedirect(redirect);
        model.addAttribute("loginForm", loginDto);
        return "auth/login";
    }

    @PostMapping("/login")
    public String login(@Valid @ModelAttribute("loginForm") LoginDto dto,
                        BindingResult result,
                        Model model,
                        HttpServletResponse response) {
        if (result.hasErrors()) {
            return "auth/login";
        }

        Optional<AuthTokensResponse> tokens = authService.login(dto.getEmail(), dto.getPassword(), dto.isRememberMe());

        if (tokens.isEmpty()) {
            model.addAttribute("error", "Invalid email or password");
            return "auth/login";
        }

        // Set cookies
        AuthTokensResponse authTokens = tokens.get();
        setAuthCookies(response, authTokens);

        // Redirect to original page or home
        String redirect = dto.getRedirect();
        if (redirect != null && !redirect.isEmpty() && redirect.startsWith("/")) {
            return "redirect:" + redirect;
        }
        return "redirect:/";
    }

    @GetMapping("/register")
    public String registerForm(Model model) {
        model.addAttribute("registerForm", new RegisterDto());
        return "auth/register";
    }

    @PostMapping("/register")
    public String register(@Valid @ModelAttribute("registerForm") RegisterDto dto,
                           BindingResult result,
                           Model model,
                           HttpServletResponse response) {
        if (result.hasErrors()) {
            return "auth/register";
        }

        if (!dto.getPassword().equals(dto.getPasswordConfirm())) {
            model.addAttribute("error", "Passwords do not match");
            return "auth/register";
        }

        Optional<AuthTokensResponse> tokens = authService.register(dto.getEmail(), dto.getPassword(), dto.getDisplayName());

        if (tokens.isEmpty()) {
            model.addAttribute("error", "Registration failed. Please try again.");
            return "auth/register";
        }

        // Set cookies
        AuthTokensResponse authTokens = tokens.get();
        setAuthCookies(response, authTokens);

        return "redirect:/";
    }

    @GetMapping("/logout")
    public String logout(HttpServletResponse response) {
        // Clear cookies
        Cookie accessCookie = new Cookie("access_token", "");
        accessCookie.setMaxAge(0);
        accessCookie.setPath("/");
        accessCookie.setHttpOnly(true);
        response.addCookie(accessCookie);

        Cookie refreshCookie = new Cookie("refresh_token", "");
        refreshCookie.setMaxAge(0);
        refreshCookie.setPath("/");
        refreshCookie.setHttpOnly(true);
        response.addCookie(refreshCookie);

        return "redirect:/";
    }

    private void setAuthCookies(HttpServletResponse response, AuthTokensResponse tokens) {
        Cookie accessCookie = new Cookie("access_token", tokens.getAccessToken());
        accessCookie.setHttpOnly(true);
        accessCookie.setPath("/");
        accessCookie.setMaxAge(24 * 60 * 60); // 24 hours
        response.addCookie(accessCookie);

        if (tokens.getRefreshToken() != null) {
            Cookie refreshCookie = new Cookie("refresh_token", tokens.getRefreshToken());
            refreshCookie.setHttpOnly(true);
            refreshCookie.setPath("/");
            refreshCookie.setMaxAge(30 * 24 * 60 * 60); // 30 days
            response.addCookie(refreshCookie);
        }
    }
}
