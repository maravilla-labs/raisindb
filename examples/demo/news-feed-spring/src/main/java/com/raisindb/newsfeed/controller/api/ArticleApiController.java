package com.raisindb.newsfeed.controller.api;

import com.raisindb.newsfeed.security.RaisinDbUserContext;
import com.raisindb.newsfeed.service.ArticleService;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.http.ResponseEntity;
import org.springframework.web.bind.annotation.*;

import java.util.Map;

/**
 * REST API controller for article operations.
 */
@RestController
@RequestMapping("/api/articles")
public class ArticleApiController {

    private final ArticleService articleService;
    private final String articlesPath;

    public ArticleApiController(ArticleService articleService,
                                @Value("${raisindb.articles-path}") String articlesPath) {
        this.articleService = articleService;
        this.articlesPath = articlesPath;
    }

    @DeleteMapping("/{category}/{slug}")
    public ResponseEntity<?> delete(@PathVariable String category,
                                    @PathVariable String slug,
                                    RaisinDbUserContext userContext) {
        if (!userContext.isAuthenticated()) {
            return ResponseEntity.status(401)
                    .body(Map.of("error", "Authentication required"));
        }

        String path = articlesPath + "/" + category + "/" + slug;

        try {
            articleService.deleteArticle(path, userContext.getAccessToken());
            return ResponseEntity.ok(Map.of("success", true));
        } catch (Exception e) {
            return ResponseEntity.status(500)
                    .body(Map.of("error", e.getMessage()));
        }
    }
}
