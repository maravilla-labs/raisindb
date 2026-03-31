package com.raisindb.newsfeed.service;

import com.raisindb.newsfeed.domain.Tag;
import com.raisindb.newsfeed.domain.TagProperties;
import com.raisindb.newsfeed.dto.TagCreateDto;
import com.raisindb.newsfeed.repository.TagRepository;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.stereotype.Service;

import java.util.*;

/**
 * Service for tag operations.
 */
@Service
public class TagService {

    private final TagRepository tagRepository;
    private final String tagsPath;

    public TagService(TagRepository tagRepository,
                      @Value("${raisindb.tags-path}") String tagsPath) {
        this.tagRepository = tagRepository;
        this.tagsPath = tagsPath;
    }

    /**
     * Get all tags as a flat list.
     */
    public List<Tag> getAllTags(String accessToken) {
        return tagRepository.findAllTags(accessToken);
    }

    /**
     * Get tags as a hierarchical tree.
     */
    public List<Tag> getTagTree(String accessToken) {
        List<Tag> flatTags = tagRepository.findAllTags(accessToken);
        return buildTree(flatTags);
    }

    /**
     * Build a hierarchical tree from flat list of tags.
     */
    private List<Tag> buildTree(List<Tag> flatTags) {
        Map<String, Tag> pathToTag = new LinkedHashMap<>();
        List<Tag> roots = new ArrayList<>();

        // First pass: index all tags by path
        for (Tag tag : flatTags) {
            pathToTag.put(tag.getPath(), tag);
        }

        // Second pass: build tree structure
        for (Tag tag : flatTags) {
            String parentPath = tag.getParentPath();

            if (parentPath.equals(tagsPath)) {
                // This is a root tag
                roots.add(tag);
            } else {
                // Find parent and add as child
                Tag parent = pathToTag.get(parentPath);
                if (parent != null) {
                    parent.getChildren().add(tag);
                } else {
                    // No parent found, treat as root
                    roots.add(tag);
                }
            }
        }

        return roots;
    }

    public Optional<Tag> getTagByPath(String path) {
        return tagRepository.findByPath(path);
    }

    public Optional<Tag> getTagById(String id) {
        return tagRepository.findById(id);
    }

    public void createTag(TagCreateDto dto, String accessToken) {
        String path = dto.getParentPath() + "/" + dto.getName();

        TagProperties properties = new TagProperties();
        properties.setLabel(dto.getLabel());
        properties.setIcon(dto.getIcon());
        properties.setColor(dto.getColor());

        tagRepository.create(path, dto.getName(), properties, accessToken);
    }

    public void updateTag(String path, TagProperties properties, String accessToken) {
        tagRepository.update(path, properties, accessToken);
    }

    public void deleteTag(String path, String accessToken) {
        tagRepository.delete(path, accessToken);
    }
}
