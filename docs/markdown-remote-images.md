# Images in Markdown preview

The in-app preview displays local images. Opening a Markdown document never requests an HTTP or HTTPS image, including badges and HTML image tags. A relative path resolves against the saved document's directory. Save an untitled document before using relative image paths.

The local SVG loader rasterizes large images in the background and caps their dimensions at the GPU texture limit. Mermaid and math use the diagram loader. Keep HTTP byte loaders and network fetching disabled in the Cargo features for the preview.
