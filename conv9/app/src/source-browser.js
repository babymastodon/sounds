(() => {
  const GROUPS = [
    ["music", "music"],
    ["voice", "voice"],
    ["nature", "nature"],
    ["places", "places"],
    ["motion", "motion"],
    ["machines", "machines"],
  ];

  const GROUP_PATTERNS = {
    music:
      /music|guitar|synth|gamelan|bagpipe|taiko|tabla|cello|piano|modular|brass|quartet|handpan|accordion|harmonica|mbira|didgeridoo|aria|bell|drum|band|flamenco|choir|techno|organ/i,
    voice:
      /speech|reading|address|announcement|oratory|protest|auction|crowd|canteen|cafeteria|audience|market|terminal|concourse|goal|match/i,
    nature:
      /surf|river|water|ice|dolphin|wildlife|fire|bee|insect|cow|livestock|rain|thunder|lightning|forest|hydrophone|scuba|hail/i,
    motion:
      /train|metro|subway|traffic|street|walk|footstep|roller|helicopter|tractor|ferry|ambulance/i,
    machines:
      /machine|industrial|electrical|laundry|letterpress|metal|radio|car.?wash|demolition|blacksmith|espresso|chainsaw|woodshop|tool|alarm|foghorn|firework|arcade|casino/i,
    places:
      /airport|marina|kitchen|cathedral|bowling|nightlife|passage|mine|salon|restaurant|room/i,
  };

  function groupFor(source) {
    const text = `${source.category} ${source.kind}`;
    return (
      GROUPS.find(([id]) => GROUP_PATTERNS[id].test(text))?.[0] ||
      "places"
    );
  }

  function formatKind(kind) {
    return kind.replaceAll("_", " ");
  }

  function createElement(tag, attributes = {}, text = "") {
    const element = document.createElement(tag);
    for (const [name, value] of Object.entries(attributes)) {
      if (name === "className") element.className = value;
      else if (name === "dataset") Object.assign(element.dataset, value);
      else element.setAttribute(name, value);
    }
    if (text) element.textContent = text;
    return element;
  }

  let cachedSpectrumPalette = null;

  function spectrumPalette() {
    if (cachedSpectrumPalette) return cachedSpectrumPalette;
    const stops = [
      [0.0, 7, 9, 10],
      [0.28, 20, 49, 54],
      [0.55, 44, 129, 124],
      [0.78, 216, 241, 80],
      [1.0, 255, 114, 92],
    ];
    cachedSpectrumPalette = new Uint8ClampedArray(256 * 3);
    for (let index = 0; index < 256; index++) {
      const value = index / 255;
      const rightIndex = stops.findIndex((stop) => value <= stop[0]);
      const right = stops[Math.max(1, rightIndex)];
      const left = stops[Math.max(0, rightIndex - 1)];
      const phase = (value - left[0]) / (right[0] - left[0]);
      for (let channel = 1; channel <= 3; channel++) {
        cachedSpectrumPalette[index * 3 + channel - 1] = Math.round(
          left[channel] + phase * (right[channel] - left[channel]),
        );
      }
    }
    return cachedSpectrumPalette;
  }

  class SourceBrowser {
    static previewCache = new Map();

    constructor(root, { role, sources, value, loadPreview, onSelect }) {
      this.root = root;
      this.root.sourceBrowser = this;
      this.role = role;
      this.sources = sources.map((source) => ({
        ...source,
        browserGroup: groupFor(source),
        searchText:
          `${source.category} ${source.kind} ${source.provider} ${source.creator}`.toLowerCase(),
      }));
      this.value = value;
      this.activeId = value;
      this.filter = "";
      this.group = "all";
      this.loadPreview = loadPreview;
      this.onSelect = onSelect;
      this.previewGeneration = 0;
      this.previewTimer = 0;
      this.previewPeaks = null;
      this.previewSpectrumMap = null;
      this.previewStatus = "";
      this.opened = false;
      this.instanceId = `source-${role.toLowerCase()}`;
      this.renderShell();
      this.setValue(value);
      this.bind();
    }

    renderShell() {
      this.root.classList.add("source-browser");
      this.trigger = createElement("button", {
        className: "source-browser-trigger",
        type: "button",
        "aria-label": `Browse clip ${this.role}`,
        "aria-haspopup": "dialog",
        "aria-expanded": "false",
        "aria-controls": `${this.instanceId}-dialog`,
        title:
          `Browse, search, filter, and preview the available sources for clip ${this.role}. ` +
          "Changing the source cancels stale rendering and starts the current method again.",
      });
      this.triggerPrimary = createElement("span", {
        className: "source-trigger-primary",
      });
      this.triggerSummary = createElement("span", {
        className: "source-trigger-summary",
      });
      this.triggerChevron = createElement(
        "span",
        { className: "source-trigger-chevron", "aria-hidden": "true" },
      );
      const chevronSvg = createElement("svg", { viewBox: "0 0 16 16" });
      chevronSvg.append(createElement("path", { d: "M3.5 6 8 10.5 12.5 6" }));
      this.triggerChevron.append(chevronSvg);
      this.trigger.append(this.triggerPrimary, this.triggerSummary, this.triggerChevron);

      this.dialog = createElement("section", {
        id: `${this.instanceId}-dialog`,
        className: "source-browser-dialog",
        role: "dialog",
        "aria-modal": "false",
        "aria-label": `Browse clip ${this.role} sources`,
        hidden: "",
      });
      const searchRow = createElement("div", { className: "source-search-row" });
      this.search = createElement("input", {
        type: "search",
        className: "source-search",
        placeholder: `search clip ${this.role.toLowerCase()}`,
        "aria-label": `Search clip ${this.role} sources`,
        autocomplete: "off",
        spellcheck: "false",
        title:
          `Filter clip ${this.role} sources by recording name, sound kind, provider, or creator. ` +
          "Use the arrow keys to enter and navigate the matching list.",
      });
      this.matchCount = createElement("output", {
        className: "source-match-count",
        "aria-live": "polite",
      });
      searchRow.append(this.search, this.matchCount);

      this.groups = createElement("div", {
        className: "source-groups",
        role: "toolbar",
        "aria-label": "Source categories",
      });
      for (const [id, label] of [["all", "all"], ...GROUPS]) {
        const button = createElement(
          "button",
          {
            type: "button",
            className: "source-group",
            dataset: { group: id },
            "aria-pressed": id === "all" ? "true" : "false",
            title:
              id === "all"
                ? "Show every available source category in the searchable list."
                : `Show only sources grouped as ${label}; search text remains active.`,
          },
          label,
        );
        this.groups.append(button);
      }

      const body = createElement("div", { className: "source-browser-body" });
      this.list = createElement("div", {
        id: `${this.instanceId}-listbox`,
        className: "source-list",
        role: "listbox",
        tabindex: "0",
        "aria-label": `Clip ${this.role} source results`,
      });
      this.preview = createElement("aside", {
        className: "source-preview",
        "aria-live": "polite",
      });
      this.previewHeading = createElement("div", { className: "source-preview-heading" });
      this.previewPlots = createElement("div", { className: "source-preview-plots" });
      this.previewWaveformFrame = createElement("div", {
        className: "source-preview-plot source-preview-waveform-frame",
      });
      this.previewCanvas = createElement("canvas", {
        className: "source-preview-waveform",
        width: "420",
        height: "116",
        "aria-label": `Waveform preview for highlighted clip ${this.role} source`,
        title:
          "Shows a lazily loaded peak waveform for the highlighted source; " +
          "moving to another result replaces stale preview work.",
      });
      this.previewWaveformFrame.append(this.previewCanvas);
      this.previewSpectrumFrame = createElement("div", {
        className: "source-preview-plot source-preview-spectrum-frame",
      });
      this.previewSpectrumCanvas = createElement("canvas", {
        className: "source-preview-spectrum",
        width: "420",
        height: "116",
        "aria-label": `FFT spectrum map for highlighted clip ${this.role} source`,
        title:
          "Shows a lazily computed FFT map: time runs left to right, low to high frequency runs " +
          "bottom to top, and color shows energy over a 72 dB range.",
      });
      this.previewSpectrumFrame.append(this.previewSpectrumCanvas);
      this.previewPlots.append(this.previewWaveformFrame, this.previewSpectrumFrame);
      this.previewStats = createElement("dl", { className: "source-preview-stats" });
      this.preview.append(
        this.previewHeading,
        this.previewPlots,
        this.previewStats,
      );
      body.append(this.list, this.preview);
      this.dialog.append(searchRow, this.groups, body);
      this.root.replaceChildren(this.trigger, this.dialog);
      this.renderList();
      this.showPreviewMetadata(this.selectedSource());
      this.previewResizeObserver = new ResizeObserver(() => {
        if (!this.opened) return;
        if (this.previewPeaks) {
          this.drawWaveform(this.previewPeaks);
          this.drawSpectrumMap(this.previewSpectrumMap);
        } else if (this.previewStatus) {
          this.drawStatus(this.previewStatus);
        }
      });
      this.previewResizeObserver.observe(this.previewPlots);
    }

    bind() {
      this.trigger.addEventListener("click", () => this.toggle());
      this.trigger.addEventListener("keydown", (event) => {
        if (["ArrowDown", "Enter", " "].includes(event.key)) {
          event.preventDefault();
          this.open();
        }
      });
      this.search.addEventListener("input", () => {
        this.filter = this.search.value.trim().toLowerCase();
        this.renderList();
      });
      this.search.addEventListener("keydown", (event) => {
        if (event.key === "ArrowDown") {
          event.preventDefault();
          this.list.focus();
          this.moveActive(0);
        } else if (event.key === "Escape") {
          event.preventDefault();
          this.close(true);
        }
      });
      this.groups.addEventListener("click", (event) => {
        const button = event.target.closest("[data-group]");
        if (!button) return;
        this.group = button.dataset.group;
        this.groups.querySelectorAll("[data-group]").forEach((candidate) => {
          candidate.setAttribute(
            "aria-pressed",
            String(candidate.dataset.group === this.group),
          );
        });
        this.renderList();
        this.search.focus();
      });
      this.list.addEventListener("click", (event) => {
        const option = event.target.closest("[role='option']");
        if (option) this.commit(option.dataset.sourceId);
      });
      this.list.addEventListener("pointerover", (event) => {
        const option = event.target.closest("[role='option']");
        if (option) this.setActive(option.dataset.sourceId, false);
      });
      this.list.addEventListener("keydown", (event) => this.handleListKey(event));
      this.dialog.addEventListener("keydown", (event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          this.close(true);
        }
      });
      document.addEventListener("pointerdown", (event) => {
        if (this.opened && !this.root.contains(event.target)) this.close(false);
      });
    }

    filteredSources() {
      const matches = this.sources.filter(
        (source) =>
          (this.group === "all" || source.browserGroup === this.group) &&
          (!this.filter || source.searchText.includes(this.filter)),
      );
      if (this.group === "all") {
        matches.sort((left, right) => {
          const groupDifference =
            GROUPS.findIndex(([id]) => id === left.browserGroup) -
            GROUPS.findIndex(([id]) => id === right.browserGroup);
          return groupDifference || left.category.localeCompare(right.category);
        });
      }
      return matches;
    }

    renderList() {
      const matches = this.filteredSources();
      this.matchCount.value = `${matches.length} / ${this.sources.length}`;
      this.matchCount.textContent = `${matches.length} / ${this.sources.length}`;
      if (!matches.some((source) => source.id === this.activeId)) {
        this.activeId = matches[0]?.id || "";
      }
      const fragment = document.createDocumentFragment();
      let previousGroup = "";
      for (const source of matches) {
        if (this.group === "all" && source.browserGroup !== previousGroup) {
          previousGroup = source.browserGroup;
          fragment.append(
            createElement(
              "div",
              { className: "source-list-group", role: "presentation" },
              previousGroup,
            ),
          );
        }
        const option = createElement("div", {
          id: `${this.instanceId}-option-${source.id}`,
          className: "source-option",
          role: "option",
          dataset: { sourceId: source.id },
          "aria-selected": String(source.id === this.value),
        });
        option.append(
          createElement("span", { className: "source-option-name" }, source.category),
          createElement(
            "span",
            { className: "source-option-duration" },
            `${source.seconds.toFixed(0)}s`,
          ),
        );
        if (source.id === this.activeId) option.classList.add("active");
        fragment.append(option);
      }
      if (!matches.length) {
        fragment.append(
          createElement(
            "div",
            { className: "source-empty", role: "status" },
            "no matching sounds",
          ),
        );
      }
      this.list.replaceChildren(fragment);
      this.syncActive();
      if (this.opened) this.scrollActiveIntoView("nearest");
      const source = this.sources.find((candidate) => candidate.id === this.activeId);
      if (source && this.opened) this.queuePreview(source);
    }

    handleListKey(event) {
      const matches = this.filteredSources();
      if (!matches.length) {
        if (event.key === "Escape") this.close(true);
        return;
      }
      const current = Math.max(
        0,
        matches.findIndex((source) => source.id === this.activeId),
      );
      const next = {
        ArrowDown: Math.min(matches.length - 1, current + 1),
        ArrowUp: Math.max(0, current - 1),
        PageDown: Math.min(matches.length - 1, current + 8),
        PageUp: Math.max(0, current - 8),
        Home: 0,
        End: matches.length - 1,
      }[event.key];
      if (next != null) {
        event.preventDefault();
        this.setActive(matches[next].id, true);
      } else if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        this.commit(this.activeId);
      } else if (event.key === "Escape") {
        event.preventDefault();
        this.close(true);
      }
    }

    moveActive(delta) {
      const matches = this.filteredSources();
      if (!matches.length) return;
      const current = matches.findIndex((source) => source.id === this.activeId);
      const index = Math.max(0, Math.min(matches.length - 1, current + delta));
      this.setActive(matches[index].id, true);
    }

    setActive(id, scroll) {
      if (!id || this.activeId === id) return;
      this.activeId = id;
      this.syncActive();
      const option = this.list.querySelector(`[data-source-id="${CSS.escape(id)}"]`);
      if (scroll) option?.scrollIntoView({ block: "nearest" });
      const source = this.sources.find((candidate) => candidate.id === id);
      if (source) this.queuePreview(source);
    }

    syncActive() {
      this.list.querySelectorAll("[role='option']").forEach((option) => {
        const active = option.dataset.sourceId === this.activeId;
        option.classList.toggle("active", active);
        option.setAttribute(
          "aria-selected",
          String(option.dataset.sourceId === this.value),
        );
      });
      if (this.activeId) {
        this.list.setAttribute(
          "aria-activedescendant",
          `${this.instanceId}-option-${this.activeId}`,
        );
      } else {
        this.list.removeAttribute("aria-activedescendant");
      }
    }

    selectedSource() {
      return this.sources.find((source) => source.id === this.value);
    }

    setValue(id) {
      const source = this.sources.find((candidate) => candidate.id === id);
      if (!source) return;
      this.value = id;
      this.activeId = id;
      this.triggerPrimary.textContent = source.category;
      this.triggerSummary.textContent = `${source.browserGroup} · ${source.seconds.toFixed(0)}s`;
      this.syncActive();
      this.showPreviewMetadata(source);
      if (this.opened) this.queuePreview(source, 0);
    }

    toggle() {
      if (this.opened) this.close(true);
      else this.open();
    }

    open() {
      if (this.opened) return;
      document.querySelectorAll(".source-browser-dialog:not([hidden])").forEach((dialog) => {
        dialog.closest(".source-browser")?.sourceBrowser?.close(false);
      });
      const selected = this.selectedSource();
      this.filter = "";
      this.search.value = "";
      this.group = selected?.browserGroup || "all";
      this.groups.querySelectorAll("[data-group]").forEach((button) => {
        button.setAttribute(
          "aria-pressed",
          String(button.dataset.group === this.group),
        );
      });
      this.opened = true;
      this.dialog.hidden = false;
      this.trigger.setAttribute("aria-expanded", "true");
      this.activeId = this.value;
      this.renderList();
      this.scrollActiveIntoView("center");
      this.search.focus();
      this.queuePreview(selected, 0);
    }

    close(returnFocus) {
      if (!this.opened) return;
      this.opened = false;
      this.dialog.hidden = true;
      this.trigger.setAttribute("aria-expanded", "false");
      clearTimeout(this.previewTimer);
      this.previewGeneration += 1;
      if (returnFocus) this.trigger.focus();
    }

    commit(id) {
      if (!id) return;
      this.setValue(id);
      this.close(true);
      this.onSelect(id);
    }

    queuePreview(source, delay = 100) {
      clearTimeout(this.previewTimer);
      if (!source) return;
      const generation = ++this.previewGeneration;
      this.showPreviewMetadata(source);
      const cached = SourceBrowser.previewCache.get(source.id);
      if (cached) {
        this.paintPreview(source, cached);
        return;
      }
      this.previewCanvas.setAttribute("aria-busy", "true");
      this.previewSpectrumCanvas.setAttribute("aria-busy", "true");
      this.drawStatus("loading waveform…");
      this.previewTimer = setTimeout(async () => {
        try {
          const preview = await this.loadPreview(source.id, 420);
          SourceBrowser.previewCache.set(source.id, preview);
          if (generation === this.previewGeneration && this.opened) {
            this.paintPreview(source, preview);
          }
        } catch (error) {
          if (generation === this.previewGeneration && this.opened) {
            this.drawStatus("preview unavailable");
            this.previewCanvas.setAttribute("aria-busy", "false");
            this.previewSpectrumCanvas.setAttribute("aria-busy", "false");
          }
        }
      }, delay);
    }

    showPreviewMetadata(source) {
      if (!source) return;
      this.previewHeading.textContent = source.category;
      this.previewStats.replaceChildren(
        this.stat("duration", `${source.seconds.toFixed(1)}s`),
        this.stat("rms", "…"),
        this.stat("peak", "…"),
        this.stat("zero-cross", "…"),
      );
    }

    stat(label, value) {
      const group = createElement("div");
      group.append(createElement("dt", {}, label), createElement("dd", {}, value));
      return group;
    }

    paintPreview(source, preview) {
      this.previewCanvas.setAttribute("aria-busy", "false");
      this.previewSpectrumCanvas.setAttribute("aria-busy", "false");
      this.drawWaveform(preview.peaks);
      this.drawSpectrumMap({
        values: preview.spectrumMap,
        columns: preview.spectrumColumns,
        rows: preview.spectrumRows,
        fftSize: preview.spectrumFftSize,
      });
      this.previewStats.replaceChildren(
        this.stat("duration", `${source.seconds.toFixed(1)}s`),
        this.stat("rms", `${preview.rmsDbfs.toFixed(1)} dB`),
        this.stat("peak", `${(preview.peak * 100).toFixed(0)}%`),
        this.stat("zero-cross", `${(preview.zeroCrossingRate * 100).toFixed(1)}%`),
      );
    }

    drawStatus(text) {
      this.previewPeaks = null;
      this.previewSpectrumMap = null;
      this.previewStatus = text;
      this.drawCanvasStatus(this.previewCanvas, text);
      this.drawCanvasStatus(
        this.previewSpectrumCanvas,
        text === "loading waveform…" ? "computing fft map…" : text,
      );
    }

    drawCanvasStatus(canvas, text) {
      const { context, width, height } = this.canvasSurface(canvas);
      context.clearRect(0, 0, width, height);
      context.fillStyle = "#181a19";
      context.fillRect(0, 0, width, height);
      context.fillStyle = "#6f716d";
      context.font = "16px monospace";
      context.textAlign = "center";
      context.textBaseline = "middle";
      context.fillText(text, width / 2, height / 2);
    }

    drawWaveform(peaks) {
      this.previewPeaks = peaks;
      this.previewStatus = "";
      const { context, width, height } = this.canvasSurface(this.previewCanvas);
      context.clearRect(0, 0, width, height);
      context.fillStyle = "#181a19";
      context.fillRect(0, 0, width, height);
      const top = peaks.map(([, maximum]) => (0.5 - maximum * 0.48) * height);
      const bottom = peaks.map(([minimum]) => (0.5 - minimum * 0.48) * height);
      context.beginPath();
      top.forEach((y, index) => {
        const x = (index / Math.max(1, peaks.length - 1)) * width;
        if (index === 0) context.moveTo(x, y);
        else context.lineTo(x, y);
      });
      for (let index = bottom.length - 1; index >= 0; index--) {
        const x = (index / Math.max(1, bottom.length - 1)) * width;
        context.lineTo(x, bottom[index]);
      }
      context.closePath();
      context.fillStyle = "#929e7c";
      context.fill();
      context.strokeStyle = "#a9b68f";
      context.lineWidth = 0.75;
      context.stroke();
    }

    drawSpectrumMap(spectrumMap) {
      this.previewSpectrumMap = spectrumMap;
      const { context, pixelWidth, pixelHeight } = this.canvasSurface(
        this.previewSpectrumCanvas,
      );
      const { values, columns, rows, fftSize } = spectrumMap || {};
      if (
        !values?.length ||
        !Number.isInteger(columns) ||
        !Number.isInteger(rows) ||
        !Number.isInteger(fftSize) ||
        values.length !== columns * rows
      ) {
        this.drawCanvasStatus(this.previewSpectrumCanvas, "fft map unavailable");
        return;
      }
      const image = context.createImageData(pixelWidth, pixelHeight);
      const palette = spectrumPalette();
      for (let y = 0; y < pixelHeight; y++) {
        const row = Math.min(rows - 1, Math.floor((y / pixelHeight) * rows));
        for (let x = 0; x < pixelWidth; x++) {
          const column = Math.min(
            columns - 1,
            Math.floor((x / pixelWidth) * columns),
          );
          const intensity = Math.max(
            0,
            Math.min(255, Math.round(values[column * rows + row] * 255)),
          );
          const colorOffset = intensity * 3;
          const pixelOffset = (y * pixelWidth + x) * 4;
          image.data[pixelOffset] = palette[colorOffset];
          image.data[pixelOffset + 1] = palette[colorOffset + 1];
          image.data[pixelOffset + 2] = palette[colorOffset + 2];
          image.data[pixelOffset + 3] = 255;
        }
      }
      context.putImageData(image, 0, 0);
      this.previewSpectrumCanvas.dataset.spectrumColumns = String(columns);
      this.previewSpectrumCanvas.dataset.spectrumRows = String(rows);
      this.previewSpectrumCanvas.dataset.fftSize = String(fftSize);
    }

    scrollActiveIntoView(block) {
      this.list
        .querySelector(`[data-source-id="${CSS.escape(this.activeId)}"]`)
        ?.scrollIntoView({ block, inline: "nearest" });
    }

    canvasSurface(canvas) {
      const bounds = canvas.getBoundingClientRect();
      const width = Math.max(1, Math.round(bounds.width));
      const height = Math.max(1, Math.round(bounds.height));
      const scale = Math.min(2, Math.max(1, window.devicePixelRatio || 1));
      const pixelWidth = Math.round(width * scale);
      const pixelHeight = Math.round(height * scale);
      if (
        canvas.width !== pixelWidth ||
        canvas.height !== pixelHeight
      ) {
        canvas.width = pixelWidth;
        canvas.height = pixelHeight;
      }
      const context = canvas.getContext("2d");
      context.setTransform(scale, 0, 0, scale, 0, 0);
      return { context, width, height, pixelWidth, pixelHeight };
    }
  }

  window.SourceBrowser = SourceBrowser;
})();
