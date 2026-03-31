<?php

use App\Http\Controllers\ArticleController;
use App\Http\Controllers\AuthController;
use App\Http\Controllers\SearchController;
use App\Http\Controllers\Settings\TagController;
use App\Http\Controllers\Settings\CategorySettingsController;
use Illuminate\Support\Facades\Route;

// Home page
Route::get('/', [ArticleController::class, 'index'])->name('home');

// Authentication
Route::prefix('auth')->name('auth.')->group(function () {
    Route::get('/login', [AuthController::class, 'showLogin'])->name('login');
    Route::post('/login', [AuthController::class, 'login']);
    Route::get('/register', [AuthController::class, 'showRegister'])->name('register');
    Route::post('/register', [AuthController::class, 'register']);
    Route::post('/logout', [AuthController::class, 'logout'])->name('logout');
});

// Search
Route::get('/search', [SearchController::class, 'index'])->name('search');

// Articles
Route::prefix('articles')->name('articles.')->group(function () {
    Route::get('/new', [ArticleController::class, 'create'])->name('create');
    Route::post('/new', [ArticleController::class, 'store'])->name('store');

    // Catch-all routes for category/article paths
    Route::get('/{path}/edit', [ArticleController::class, 'edit'])
        ->where('path', '.*')
        ->name('edit');

    Route::put('/{path}', [ArticleController::class, 'update'])
        ->where('path', '.*')
        ->name('update');

    Route::delete('/{path}', [ArticleController::class, 'destroy'])
        ->where('path', '.*')
        ->name('destroy');

    Route::get('/{path}/move', [ArticleController::class, 'showMove'])
        ->where('path', '.*')
        ->name('move');

    Route::put('/{path}/move', [ArticleController::class, 'move'])
        ->where('path', '.*')
        ->name('move.update');

    // This catches both category pages and article pages
    Route::get('/{path}', [ArticleController::class, 'show'])
        ->where('path', '.*')
        ->name('show');
});

// Settings
Route::prefix('settings')->name('settings.')->group(function () {
    Route::prefix('categories')->name('categories.')->group(function () {
        Route::get('/', [CategorySettingsController::class, 'index'])->name('index');
        Route::post('/', [CategorySettingsController::class, 'store'])->name('store');
        Route::put('/{slug}', [CategorySettingsController::class, 'update'])->name('update');
        Route::delete('/{slug}', [CategorySettingsController::class, 'destroy'])->name('destroy');
    });

    Route::prefix('tags')->name('tags.')->group(function () {
        Route::get('/', [TagController::class, 'index'])->name('index');
        Route::post('/', [TagController::class, 'store'])->name('store');
        Route::put('/{path}', [TagController::class, 'update'])
            ->where('path', '.*')
            ->name('update');
        Route::delete('/{path}', [TagController::class, 'destroy'])
            ->where('path', '.*')
            ->name('destroy');
    });
});
