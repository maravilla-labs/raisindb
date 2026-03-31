package com.raisindb.newsfeed.controller;

import com.raisindb.newsfeed.domain.*;
import com.raisindb.newsfeed.dto.ArticleCreateDto;
import com.raisindb.newsfeed.dto.ArticleUpdateDto;
import com.raisindb.newsfeed.repository.GraphRepository;
import com.raisindb.newsfeed.security.RaisinDbUserContext;
import com.raisindb.newsfeed.service.ArticleService;
import com.raisindb.newsfeed.service.CategoryService;
import com.raisindb.newsfeed.service.TagService;
import com.raisindb.newsfeed.util.PathUtils;
import com.raisindb.newsfeed.util.RelationTypes;
import jakarta.validation.Valid;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.stereotype.Controller;
import org.springframework.ui.Model;
import org.springframework.validation.BindingResult;
import org.springframework.web.bind.annotation.*;
import org.springframework.web.servlet.mvc.support.RedirectAttributes;

import java.util.List;
import java.util.Optional;

/**
 * Controller for article operations.
 */
@Controller
@RequestMapping("/articles")
public class ArticleController {

    private final ArticleService articleService;
    private final CategoryService categoryService;
    private final TagService tagService;
    private final GraphRepository graphRepository;
    private final String articlesPath;

    public ArticleController(ArticleService articleService,
                             CategoryService categoryService,
                             TagService tagService,
                             GraphRepository graphRepository,
                             @Value("${raisindb.articles-path}") String articlesPath) {
        this.articleService = articleService;
        this.categoryService = categoryService;
        this.tagService = tagService;
        this.graphRepository = graphRepository;
        this.articlesPath = articlesPath;
    }

    /**
     * View category listing.
     */
    @GetMapping("/{category}")
    public String viewCategory(@PathVariable String category, Model model,
                               RaisinDbUserContext userContext) {
        String accessToken = userContext.getAccessToken();

        Optional<Category> categoryOpt = categoryService.getCategoryBySlug(category, accessToken);
        if (categoryOpt.isEmpty()) {
            return "error/404";
        }

        Category cat = categoryOpt.get();

        model.addAttribute("category", cat);
        model.addAttribute("articles", articleService.getArticlesByCategory(cat.getPath(), accessToken));
        model.addAttribute("categories", categoryService.getAllCategories(accessToken));
        model.addAttribute("tags", tagService.getTagTree(accessToken));
        model.addAttribute("userContext", userContext);

        return "category/view";
    }

    /**
     * View single article.
     */
    @GetMapping("/{category}/{slug}")
    public String viewArticle(@PathVariable String category,
                              @PathVariable String slug,
                              Model model,
                              RaisinDbUserContext userContext) {
        String path = articlesPath + "/" + category + "/" + slug;

        ArticleService.ArticleViewData viewData = articleService.getArticleViewData(path);
        if (viewData == null) {
            return "error/404";
        }

        String accessToken = userContext.getAccessToken();

        model.addAttribute("article", viewData.getArticle());
        model.addAttribute("graphData", viewData.getGraphData());
        model.addAttribute("related", viewData.getRelatedArticles());
        model.addAttribute("categories", categoryService.getAllCategories(accessToken));
        model.addAttribute("tags", tagService.getTagTree(accessToken));
        model.addAttribute("userContext", userContext);

        return "article/view";
    }

    /**
     * Create article form.
     */
    @GetMapping("/new")
    public String createForm(Model model, RaisinDbUserContext userContext) {
        if (!userContext.isAuthenticated()) {
            return "redirect:/auth/login?redirect=/articles/new";
        }

        String accessToken = userContext.getAccessToken();

        model.addAttribute("articleForm", new ArticleCreateDto());
        model.addAttribute("categories", categoryService.getAllCategories(accessToken));
        model.addAttribute("tags", tagService.getTagTree(accessToken));
        model.addAttribute("userContext", userContext);

        return "article/create";
    }

    /**
     * Create article submit.
     */
    @PostMapping("/new")
    public String create(@Valid @ModelAttribute("articleForm") ArticleCreateDto dto,
                         BindingResult result,
                         Model model,
                         RaisinDbUserContext userContext,
                         RedirectAttributes redirectAttributes) {
        if (!userContext.isAuthenticated()) {
            return "redirect:/auth/login";
        }

        if (result.hasErrors()) {
            model.addAttribute("categories", categoryService.getAllCategories(userContext.getAccessToken()));
            model.addAttribute("tags", tagService.getTagTree(userContext.getAccessToken()));
            model.addAttribute("userContext", userContext);
            return "article/create";
        }

        String author = userContext.getUser().getDisplayNameOrEmail();

        articleService.createArticle(dto, author, userContext.getAccessToken());

        redirectAttributes.addFlashAttribute("success", "Article created successfully");
        return "redirect:/articles/" + dto.getCategory() + "/" + dto.getSlug();
    }

    /**
     * Edit article form.
     */
    @GetMapping("/{category}/{slug}/edit")
    public String editForm(@PathVariable String category,
                           @PathVariable String slug,
                           Model model,
                           RaisinDbUserContext userContext) {
        if (!userContext.isAuthenticated()) {
            return "redirect:/auth/login";
        }

        String path = articlesPath + "/" + category + "/" + slug;
        Optional<Article> articleOpt = articleService.getArticleByPath(path);
        if (articleOpt.isEmpty()) {
            return "error/404";
        }

        Article article = articleOpt.get();
        String accessToken = userContext.getAccessToken();

        // Get all articles for connection picker
        List<Article> allArticles = articleService.getAllArticlesExcept(path, accessToken);

        // Get incoming connections
        List<IncomingConnection> incomingConnections = graphRepository.findIncomingConnections(path);

        model.addAttribute("article", article);
        model.addAttribute("currentCategory", category);
        model.addAttribute("categories", categoryService.getAllCategories(accessToken));
        model.addAttribute("tags", tagService.getTagTree(accessToken));
        model.addAttribute("flatTags", tagService.getAllTags(accessToken));
        model.addAttribute("availableArticles", allArticles);
        model.addAttribute("incomingConnections", incomingConnections);
        model.addAttribute("relationTypes", RelationTypes.RELATION_TYPE_META);
        model.addAttribute("userContext", userContext);

        return "article/edit";
    }

    /**
     * Edit article submit.
     */
    @PostMapping("/{category}/{slug}/edit")
    public String update(@PathVariable String category,
                         @PathVariable String slug,
                         @Valid @ModelAttribute ArticleUpdateDto dto,
                         BindingResult result,
                         Model model,
                         RaisinDbUserContext userContext,
                         RedirectAttributes redirectAttributes) {
        if (!userContext.isAuthenticated()) {
            return "redirect:/auth/login";
        }

        if (result.hasErrors()) {
            String path = articlesPath + "/" + category + "/" + slug;
            articleService.getArticleByPath(path).ifPresent(article -> model.addAttribute("article", article));
            model.addAttribute("categories", categoryService.getAllCategories(userContext.getAccessToken()));
            model.addAttribute("tags", tagService.getTagTree(userContext.getAccessToken()));
            model.addAttribute("userContext", userContext);
            return "article/edit";
        }

        String originalPath = articlesPath + "/" + category + "/" + slug;
        articleService.updateArticle(originalPath, dto, userContext.getAccessToken());

        redirectAttributes.addFlashAttribute("success", "Article updated successfully");
        return "redirect:/articles/" + dto.getCategory() + "/" + dto.getSlug();
    }
}
