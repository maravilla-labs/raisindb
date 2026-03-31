package com.raisindb.newsfeed.controller;

import com.raisindb.newsfeed.security.RaisinDbUserContext;
import com.raisindb.newsfeed.service.ArticleService;
import com.raisindb.newsfeed.service.CategoryService;
import com.raisindb.newsfeed.service.TagService;
import org.springframework.stereotype.Controller;
import org.springframework.ui.Model;
import org.springframework.web.bind.annotation.GetMapping;

/**
 * Controller for the home page.
 */
@Controller
public class HomeController {

    private final ArticleService articleService;
    private final CategoryService categoryService;
    private final TagService tagService;

    public HomeController(ArticleService articleService,
                          CategoryService categoryService,
                          TagService tagService) {
        this.articleService = articleService;
        this.categoryService = categoryService;
        this.tagService = tagService;
    }

    @GetMapping("/")
    public String home(Model model, RaisinDbUserContext userContext) {
        String accessToken = userContext.getAccessToken();

        model.addAttribute("featured", articleService.getFeaturedArticles(accessToken));
        model.addAttribute("recent", articleService.getRecentArticles(accessToken));
        model.addAttribute("categories", categoryService.getAllCategories(accessToken));
        model.addAttribute("tags", tagService.getTagTree(accessToken));
        model.addAttribute("userContext", userContext);

        return "home";
    }
}
