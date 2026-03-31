<?php

namespace App\Http\Controllers;

use App\Services\RaisinDB\RaisinAuthService;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\Validator;

class AuthController extends Controller
{
    protected RaisinAuthService $authService;

    public function __construct(RaisinAuthService $authService)
    {
        $this->authService = $authService;
    }

    /**
     * Show login form
     */
    public function showLogin(Request $request)
    {
        // Redirect if already logged in
        if ($this->authService->getSessionUser()) {
            return redirect('/');
        }

        return view('auth.login', [
            'redirect' => $request->query('redirect', '/'),
        ]);
    }

    /**
     * Handle login submission
     */
    public function login(Request $request)
    {
        $validator = Validator::make($request->all(), [
            'email' => 'required|email',
            'password' => 'required|min:6',
        ]);

        if ($validator->fails()) {
            return back()
                ->withErrors($validator)
                ->withInput($request->except('password'));
        }

        $result = $this->authService->login(
            $request->input('email'),
            $request->input('password'),
            $request->boolean('remember_me')
        );

        if (!$result['success']) {
            return back()
                ->withErrors(['email' => $result['error']['message']])
                ->withInput($request->except('password'));
        }

        $cookies = $this->authService->createAuthCookies($result['tokens']);
        $redirect = $request->input('redirect', '/');

        $response = redirect($redirect)->with('success', 'Welcome back!');

        foreach ($cookies as $cookie) {
            $response->withCookie($cookie);
        }

        return $response;
    }

    /**
     * Show registration form
     */
    public function showRegister(Request $request)
    {
        // Redirect if already logged in
        if ($this->authService->getSessionUser()) {
            return redirect('/');
        }

        return view('auth.register', [
            'redirect' => $request->query('redirect', '/'),
        ]);
    }

    /**
     * Handle registration submission
     */
    public function register(Request $request)
    {
        $validator = Validator::make($request->all(), [
            'email' => 'required|email',
            'password' => 'required|min:8|confirmed',
            'display_name' => 'nullable|string|max:255',
        ], [
            'password.confirmed' => 'The password confirmation does not match.',
            'password.min' => 'Password must be at least 8 characters.',
        ]);

        if ($validator->fails()) {
            return back()
                ->withErrors($validator)
                ->withInput($request->except(['password', 'password_confirmation']));
        }

        $result = $this->authService->register(
            $request->input('email'),
            $request->input('password'),
            $request->input('display_name')
        );

        if (!$result['success']) {
            return back()
                ->withErrors(['email' => $result['error']['message']])
                ->withInput($request->except(['password', 'password_confirmation']));
        }

        $cookies = $this->authService->createAuthCookies($result['tokens']);
        $redirect = $request->input('redirect', '/');

        $response = redirect($redirect)->with('success', 'Account created successfully!');

        foreach ($cookies as $cookie) {
            $response->withCookie($cookie);
        }

        return $response;
    }

    /**
     * Handle logout
     */
    public function logout()
    {
        $cookies = $this->authService->createLogoutCookies();

        $response = redirect('/')->with('success', 'You have been logged out.');

        foreach ($cookies as $cookie) {
            $response->withCookie($cookie);
        }

        return $response;
    }
}
