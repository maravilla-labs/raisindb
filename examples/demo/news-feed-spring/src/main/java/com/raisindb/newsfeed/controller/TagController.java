package com.raisindb.newsfeed.controller;

import com.raisindb.newsfeed.domain.TagProperties;
import com.raisindb.newsfeed.dto.TagCreateDto;
import com.raisindb.newsfeed.security.RaisinDbUserContext;
import com.raisindb.newsfeed.service.CategoryService;
import com.raisindb.newsfeed.service.TagService;
import jakarta.validation.Valid;
import org.springframework.stereotype.Controller;
import org.springframework.ui.Model;
import org.springframework.validation.BindingResult;
import org.springframework.web.bind.annotation.*;
import org.springframework.web.servlet.mvc.support.RedirectAttributes;

/**
 * Controller for tag management.
 */
@Controller
@RequestMapping("/settings/tags")
public class TagController {

    private final TagService tagService;
    private final CategoryService categoryService;

    public TagController(TagService tagService, CategoryService categoryService) {
        this.tagService = tagService;
        this.categoryService = categoryService;
    }

    @GetMapping
    public String listTags(Model model, RaisinDbUserContext userContext) {
        if (!userContext.isAuthenticated()) {
            return "redirect:/auth/login?redirect=/settings/tags";
        }

        String accessToken = userContext.getAccessToken();

        model.addAttribute("tags", tagService.getTagTree(accessToken));
        model.addAttribute("flatTags", tagService.getAllTags(accessToken));
        model.addAttribute("categories", categoryService.getAllCategories(accessToken));
        model.addAttribute("tagForm", new TagCreateDto());
        model.addAttribute("userContext", userContext);

        return "tag/settings";
    }

    @PostMapping("/create")
    public String createTag(@Valid @ModelAttribute("tagForm") TagCreateDto dto,
                            BindingResult result,
                            RaisinDbUserContext userContext,
                            RedirectAttributes redirectAttributes) {
        if (!userContext.isAuthenticated()) {
            return "redirect:/auth/login";
        }

        if (result.hasErrors()) {
            redirectAttributes.addFlashAttribute("error", "Please fill in all required fields");
            return "redirect:/settings/tags";
        }

        tagService.createTag(dto, userContext.getAccessToken());
        redirectAttributes.addFlashAttribute("success", "Tag created successfully");

        return "redirect:/settings/tags";
    }

    @PostMapping("/{id}/update")
    public String updateTag(@PathVariable String id,
                            @RequestParam String label,
                            @RequestParam(required = false) String icon,
                            @RequestParam(required = false) String color,
                            RaisinDbUserContext userContext,
                            RedirectAttributes redirectAttributes) {
        if (!userContext.isAuthenticated()) {
            return "redirect:/auth/login";
        }

        tagService.getTagById(id).ifPresent(tag -> {
            TagProperties properties = new TagProperties();
            properties.setLabel(label);
            properties.setIcon(icon);
            properties.setColor(color);
            tagService.updateTag(tag.getPath(), properties, userContext.getAccessToken());
        });

        redirectAttributes.addFlashAttribute("success", "Tag updated successfully");
        return "redirect:/settings/tags";
    }

    @PostMapping("/{id}/delete")
    public String deleteTag(@PathVariable String id,
                            RaisinDbUserContext userContext,
                            RedirectAttributes redirectAttributes) {
        if (!userContext.isAuthenticated()) {
            return "redirect:/auth/login";
        }

        tagService.getTagById(id).ifPresent(tag -> {
            tagService.deleteTag(tag.getPath(), userContext.getAccessToken());
        });

        redirectAttributes.addFlashAttribute("success", "Tag deleted successfully");
        return "redirect:/settings/tags";
    }
}
