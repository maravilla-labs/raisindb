package com.raisindb.newsfeed.service;

import com.raisindb.newsfeed.domain.Category;
import com.raisindb.newsfeed.domain.CategoryProperties;
import com.raisindb.newsfeed.repository.CategoryRepository;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.stereotype.Service;

import java.util.List;
import java.util.Optional;

/**
 * Service for category operations.
 */
@Service
public class CategoryService {

    private final CategoryRepository categoryRepository;
    private final String articlesPath;

    public CategoryService(CategoryRepository categoryRepository,
                           @Value("${raisindb.articles-path}") String articlesPath) {
        this.categoryRepository = categoryRepository;
        this.articlesPath = articlesPath;
    }

    public List<Category> getAllCategories(String accessToken) {
        return categoryRepository.findAllCategories(accessToken);
    }

    public Optional<Category> getCategoryBySlug(String slug, String accessToken) {
        return categoryRepository.findBySlug(slug, accessToken);
    }

    public Optional<Category> getCategoryByPath(String path) {
        return categoryRepository.findByPath(path);
    }

    public void createCategory(String slug, String name, CategoryProperties properties, String accessToken) {
        String path = articlesPath + "/" + slug;
        categoryRepository.create(path, name, properties, accessToken);
    }

    public void updateCategory(String path, String name, CategoryProperties properties, String accessToken) {
        categoryRepository.update(path, name, properties, accessToken);
    }

    public void deleteCategory(String path, String accessToken) {
        categoryRepository.delete(path, accessToken);
    }
}
