package com.raisindb.newsfeed.util;

import com.vladsch.flexmark.html.HtmlRenderer;
import com.vladsch.flexmark.parser.Parser;
import com.vladsch.flexmark.util.ast.Node;
import com.vladsch.flexmark.util.data.MutableDataSet;
import com.vladsch.flexmark.ext.tables.TablesExtension;
import com.vladsch.flexmark.ext.gfm.strikethrough.StrikethroughExtension;
import com.vladsch.flexmark.ext.autolink.AutolinkExtension;
import org.springframework.stereotype.Service;

import java.util.Arrays;

/**
 * Service for converting Markdown to HTML using Flexmark.
 */
@Service("markdown")
public class MarkdownService {

    private final Parser parser;
    private final HtmlRenderer renderer;

    public MarkdownService() {
        MutableDataSet options = new MutableDataSet();

        // Enable GitHub Flavored Markdown extensions
        options.set(Parser.EXTENSIONS, Arrays.asList(
                TablesExtension.create(),
                StrikethroughExtension.create(),
                AutolinkExtension.create()
        ));

        // Configure HTML renderer
        options.set(HtmlRenderer.SOFT_BREAK, "<br />\n");

        this.parser = Parser.builder(options).build();
        this.renderer = HtmlRenderer.builder(options).build();
    }

    /**
     * Convert Markdown text to HTML.
     */
    public String toHtml(String markdown) {
        if (markdown == null || markdown.isEmpty()) {
            return "";
        }

        Node document = parser.parse(markdown);
        return renderer.render(document);
    }
}
