<?php

namespace App\Http\Middleware;

use App\Services\RaisinDB\RaisinAuthService;
use App\Services\RaisinDB\RaisinQueryBuilder;
use Closure;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\View;
use Symfony\Component\HttpFoundation\Response;

/**
 * Middleware to set identity user context for RaisinDB queries.
 *
 * Extracts the JWT access token from the Authorization header or cookies
 * and sets it on RaisinQueryBuilder for row-level security.
 * Also shares the current user with all views.
 */
class RaisinUserContext
{
    protected RaisinAuthService $authService;

    public function __construct(RaisinAuthService $authService)
    {
        $this->authService = $authService;
    }

    /**
     * Handle an incoming request.
     *
     * @param  \Closure(\Illuminate\Http\Request): (\Symfony\Component\HttpFoundation\Response)  $next
     */
    public function handle(Request $request, Closure $next): Response
    {
        // Extract Bearer token from Authorization header or cookies
        $token = $this->extractBearerToken($request);

        if ($token) {
            // Set the token for RaisinDB queries (enables row-level security)
            RaisinQueryBuilder::setUserToken($token);

            // Get user from token and share with views
            $user = $this->authService->getUserFromToken($token);
            View::share('currentUser', $user);
        } else {
            View::share('currentUser', null);
        }

        try {
            return $next($request);
        } finally {
            // Always clear the token after the request
            RaisinQueryBuilder::clearUserToken();
        }
    }

    /**
     * Extract Bearer token from Authorization header or cookies
     */
    protected function extractBearerToken(Request $request): ?string
    {
        $header = $request->header('Authorization');

        if ($header && str_starts_with($header, 'Bearer ')) {
            return substr($header, 7);
        }

        // Also check for token in cookies (for web sessions)
        $cookieToken = $request->cookie('access_token');
        if ($cookieToken) {
            return $cookieToken;
        }

        return null;
    }
}
