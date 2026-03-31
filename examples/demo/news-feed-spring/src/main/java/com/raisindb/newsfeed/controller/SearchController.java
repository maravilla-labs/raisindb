package com.raisindb.newsfeed.controller;

import com.raisindb.newsfeed.domain.Article;
import com.raisindb.newsfeed.security.RaisinDbUserContext;
import com.raisindb.newsfeed.service.ArticleService;
import com.raisindb.newsfeed.service.CategoryService;
import com.raisindb.newsfeed.service.TagService;
import org.springframework.stereotype.Controller;
import org.springframework.ui.Model;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RequestMapping;
import org.springframework.web.bind.annotation.RequestParam;

import java.util.Collections;
import java.util.List;

/**
 * Controller for search operations.
 */
@Controller
@RequestMapping("/search")
public class SearchController {

    private final ArticleService articleService;
    private final CategoryService categoryService;
    private final TagService tagService;

    public SearchController(ArticleService articleService,
                            CategoryService categoryService,
                            TagService tagService) {
        this.articleService = articleService;
        this.categoryService = categoryService;
        this.tagService = tagService;
    }

    @GetMapping
    public String search(@RequestParam(required = false) String q,
                         @RequestParam(required = false) String tag,
                         Model model,
                         RaisinDbUserContext userContext) {
        String accessToken = userContext.getAccessToken();

        List<Article> articles;

        if (tag != null && !tag.isBlank()) {
            // Search by tag using REFERENCES predicate
            articles = articleService.searchByTag(tag);
        } else if (q != null && !q.isBlank()) {
            // Keyword search
            articles = articleService.searchByKeyword(q);
        } else {
            articles = Collections.emptyList();
        }

        model.addAttribute("query", q);
        model.addAttribute("tag", tag);
        model.addAttribute("articles", articles);
        model.addAttribute("categories", categoryService.getAllCategories(accessToken));
        model.addAttribute("tags", tagService.getTagTree(accessToken));
        model.addAttribute("userContext", userContext);

        return "search/results";
    }
}
