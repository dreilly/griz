#include <ghostty/vt.h>

#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define GRIZ_CELL_BOLD (1u << 0)
#define GRIZ_CELL_ITALIC (1u << 1)
#define GRIZ_CELL_UNDERLINE (1u << 2)

struct GrizVtCell {
    size_t text_offset;
    size_t text_len;
    uint8_t width;
    uint8_t fg[3];
    uint8_t bg[3];
    uint8_t flags;
};

struct GrizVtSnapshot {
    uint16_t cols;
    uint16_t rows;
    uint16_t cursor_x;
    uint16_t cursor_y;
    uint8_t cursor_visible;
    uint8_t application_cursor;
    uint8_t bracketed_paste;
    uint8_t mouse_tracking;
    struct GrizVtCell* cells;
    size_t cells_len;
    uint8_t* text;
    size_t text_len;
    size_t text_cap;
    uint8_t* title;
    size_t title_len;
};

struct GrizVt {
    GhosttyTerminal terminal;
    GhosttyRenderState render_state;
    GhosttyRenderStateRowIterator row_iterator;
    GhosttyRenderStateRowCells row_cells;
    GhosttyMouseEncoder mouse_encoder;
    GhosttyMouseEvent mouse_event;
    bool mouse_configured;
    uint16_t mouse_modes;
    uint16_t mouse_cols;
    uint16_t mouse_rows;
    uint16_t mouse_cell_width;
    uint16_t mouse_cell_height;
    uint8_t* responses;
    size_t responses_len;
    size_t responses_cap;
    void (*program_status)(void*, const GhosttyTerminalProgramStatus*);
    void (*prompt_start)(void*);
    void* status_userdata;
};

static void griz_program_status(
    GhosttyTerminal terminal,
    void* userdata,
    const GhosttyTerminalProgramStatus* report
) {
    (void)terminal;
    struct GrizVt* vt = userdata;
    vt->program_status(vt->status_userdata, report);
}

static void griz_semantic_prompt(
    GhosttyTerminal terminal,
    void* userdata,
    const GhosttyTerminalSemanticPrompt* event
) {
    (void)terminal;
    struct GrizVt* vt = userdata;
    if (event->kind == GHOSTTY_SEMANTIC_PROMPT_PROMPT_START) {
        vt->prompt_start(vt->status_userdata);
    }
}

static void griz_write_pty(
    GhosttyTerminal terminal,
    void* userdata,
    const uint8_t* data,
    size_t len
) {
    (void)terminal;
    struct GrizVt* vt = userdata;
    if (vt == NULL || data == NULL || len == 0 || len > SIZE_MAX - vt->responses_len) {
        return;
    }

    size_t needed = vt->responses_len + len;
    if (needed > vt->responses_cap) {
        size_t capacity = vt->responses_cap == 0 ? 256 : vt->responses_cap;
        while (capacity < needed) {
            if (capacity > SIZE_MAX / 2) {
                capacity = needed;
                break;
            }
            capacity *= 2;
        }
        uint8_t* responses = realloc(vt->responses, capacity);
        if (responses == NULL) {
            return;
        }
        vt->responses = responses;
        vt->responses_cap = capacity;
    }

    memcpy(vt->responses + vt->responses_len, data, len);
    vt->responses_len = needed;
}

static void griz_vt_free_partial(struct GrizVt* vt) {
    if (vt == NULL) {
        return;
    }
    free(vt->responses);
    ghostty_mouse_event_free(vt->mouse_event);
    ghostty_mouse_encoder_free(vt->mouse_encoder);
    ghostty_render_state_row_cells_free(vt->row_cells);
    ghostty_render_state_row_iterator_free(vt->row_iterator);
    ghostty_render_state_free(vt->render_state);
    ghostty_terminal_free(vt->terminal);
    free(vt);
}

int griz_vt_new(
    uint16_t cols,
    uint16_t rows,
    struct GrizVt** out,
    void (*program_status)(void*, const GhosttyTerminalProgramStatus*),
    void (*prompt_start)(void*),
    void* status_userdata
) {
    if (out == NULL) {
        return GHOSTTY_INVALID_VALUE;
    }
    *out = NULL;

    struct GrizVt* vt = calloc(1, sizeof(*vt));
    if (vt == NULL) {
        return GHOSTTY_OUT_OF_MEMORY;
    }

    vt->program_status = program_status;
    vt->prompt_start = prompt_start;
    vt->status_userdata = status_userdata;
    GhosttyResult result = ghostty_terminal_new(NULL, &vt->terminal, cols, rows);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_render_state_new(NULL, &vt->render_state);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_render_state_row_iterator_new(NULL, &vt->row_iterator);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_render_state_row_cells_new(NULL, &vt->row_cells);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_mouse_encoder_new(NULL, &vt->mouse_encoder);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    bool track_last_cell = true;
    result = ghostty_mouse_encoder_set(vt->mouse_encoder,
        GHOSTTY_MOUSE_ENCODER_OPT_TRACK_LAST_CELL, &track_last_cell);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_mouse_event_new(NULL, &vt->mouse_event);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }

    result = ghostty_terminal_set(vt->terminal, GHOSTTY_TERMINAL_OPT_USERDATA, vt);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_terminal_set(
        vt->terminal,
        GHOSTTY_TERMINAL_OPT_WRITE_PTY,
        (const void*)griz_write_pty
    );
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }

    // Registering the native status hook also enables OSC 7501 feature replies
    // through write_pty, in stream order before a following DA response.
    result = ghostty_terminal_set(vt->terminal, GHOSTTY_TERMINAL_OPT_PROGRAM_STATUS,
                                  (const void*)griz_program_status);
    if (result == GHOSTTY_SUCCESS) {
        result = ghostty_terminal_set(vt->terminal, GHOSTTY_TERMINAL_OPT_SEMANTIC_PROMPT,
                                      (const void*)griz_semantic_prompt);
    }
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }

    *out = vt;
    return GHOSTTY_SUCCESS;
}

void griz_vt_free(struct GrizVt* vt) {
    griz_vt_free_partial(vt);
}

void griz_vt_write(struct GrizVt* vt, const uint8_t* data, size_t len) {
    if (vt == NULL) {
        return;
    }
    ghostty_terminal_vt_write(vt->terminal, data, len);
}

int griz_vt_resize(struct GrizVt* vt, uint16_t cols, uint16_t rows) {
    if (vt == NULL) {
        return GHOSTTY_INVALID_VALUE;
    }
    return ghostty_terminal_resize(vt->terminal, cols, rows, 0, 0);
}

static int griz_mode(struct GrizVt* vt, GhosttyMode mode, uint8_t* out);

// set_from_terminal clears motion deduplication. Refresh it only when modes
// actually change, not on every input event or ordinary application redraw.
static int griz_mouse_sync_modes(struct GrizVt* vt) {
    const GhosttyMode modes[] = {
        GHOSTTY_MODE_MOUSE_EVENT_X10,
        GHOSTTY_MODE_MOUSE_EVENT_NORMAL,
        GHOSTTY_MODE_MOUSE_EVENT_BUTTON,
        GHOSTTY_MODE_MOUSE_EVENT_ANY,
        GHOSTTY_MODE_MOUSE_FORMAT_UTF8,
        GHOSTTY_MODE_MOUSE_FORMAT_SGR,
        GHOSTTY_MODE_MOUSE_FORMAT_URXVT,
        GHOSTTY_MODE_MOUSE_FORMAT_SGR_PIXELS,
    };
    uint16_t bits = 0;
    for (size_t i = 0; i < sizeof(modes) / sizeof(modes[0]); i++) {
        uint8_t enabled = 0;
        int result = griz_mode(vt, modes[i], &enabled);
        if (result != GHOSTTY_SUCCESS) return result;
        if (enabled) bits |= (uint16_t)(1u << i);
    }
    if (vt->mouse_configured && vt->mouse_modes == bits) return GHOSTTY_SUCCESS;
    int result = ghostty_mouse_encoder_set_from_terminal(vt->mouse_encoder, vt->terminal);
    if (result == GHOSTTY_SUCCESS) {
        vt->mouse_modes = bits;
        vt->mouse_configured = true;
    }
    return result;
}

int griz_vt_mouse_input(
    struct GrizVt* vt,
    uint8_t action,
    int8_t button,
    float x,
    float y,
    uint16_t cell_width,
    uint16_t cell_height,
    uint8_t shift,
    uint8_t alt,
    uint8_t ctrl,
    uint8_t** out_data,
    size_t* out_len
) {
    if (vt == NULL || out_data == NULL || out_len == NULL) {
        return GHOSTTY_INVALID_VALUE;
    }
    *out_data = NULL;
    *out_len = 0;

    GhosttyMouseAction ghostty_action;
    switch (action) {
    case 0: ghostty_action = GHOSTTY_MOUSE_ACTION_PRESS; break;
    case 1: ghostty_action = GHOSTTY_MOUSE_ACTION_RELEASE; break;
    case 2: ghostty_action = GHOSTTY_MOUSE_ACTION_MOTION; break;
    default: return GHOSTTY_INVALID_VALUE;
    }

    GhosttyResult result = griz_mouse_sync_modes(vt);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    uint16_t cols = 0;
    uint16_t rows = 0;
    result = ghostty_terminal_get(vt->terminal, GHOSTTY_TERMINAL_DATA_COLS, &cols);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }
    result = ghostty_terminal_get(vt->terminal, GHOSTTY_TERMINAL_DATA_ROWS, &rows);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }
    if (vt->mouse_cols != cols || vt->mouse_rows != rows ||
        vt->mouse_cell_width != cell_width || vt->mouse_cell_height != cell_height) {
        GhosttyMouseEncoderSize size = GHOSTTY_INIT_SIZED(GhosttyMouseEncoderSize);
        size.screen_width = (uint32_t)cols * cell_width;
        size.screen_height = (uint32_t)rows * cell_height;
        size.cell_width = cell_width;
        size.cell_height = cell_height;
        result = ghostty_mouse_encoder_set(
            vt->mouse_encoder, GHOSTTY_MOUSE_ENCODER_OPT_SIZE, &size);
        if (result != GHOSTTY_SUCCESS) return result;
        result = ghostty_mouse_encoder_reset(vt->mouse_encoder);
        if (result != GHOSTTY_SUCCESS) return result;
        vt->mouse_cols = cols;
        vt->mouse_rows = rows;
        vt->mouse_cell_width = cell_width;
        vt->mouse_cell_height = cell_height;
    }

    bool any_button_pressed = button >= 0 && ghostty_action != GHOSTTY_MOUSE_ACTION_RELEASE;
    result = ghostty_mouse_encoder_set(
        vt->mouse_encoder,
        GHOSTTY_MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED,
        &any_button_pressed
    );
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    result = ghostty_mouse_event_set_action(vt->mouse_event, ghostty_action);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }
    if (button < 0) {
        result = ghostty_mouse_event_clear_button(vt->mouse_event);
    } else {
        GhosttyMouseButton ghostty_button;
        switch (button) {
        case 0: ghostty_button = GHOSTTY_MOUSE_BUTTON_LEFT; break;
        case 1: ghostty_button = GHOSTTY_MOUSE_BUTTON_RIGHT; break;
        case 2: ghostty_button = GHOSTTY_MOUSE_BUTTON_MIDDLE; break;
        case 3: ghostty_button = GHOSTTY_MOUSE_BUTTON_FOUR; break;
        case 4: ghostty_button = GHOSTTY_MOUSE_BUTTON_FIVE; break;
        default: return GHOSTTY_INVALID_VALUE;
        }
        result = ghostty_mouse_event_set_button(vt->mouse_event, ghostty_button);
    }
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    GhosttyMods mods = 0;
    if (shift) mods |= GHOSTTY_MODS_SHIFT;
    if (alt) mods |= GHOSTTY_MODS_ALT;
    if (ctrl) mods |= GHOSTTY_MODS_CTRL;
    result = ghostty_mouse_event_set_mods(vt->mouse_event, mods);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }
    GhosttyMousePosition position = {.x = x, .y = y};
    result = ghostty_mouse_event_set_position(vt->mouse_event, position);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    size_t needed = 0;
    result = ghostty_mouse_encoder_encode(
        vt->mouse_encoder,
        vt->mouse_event,
        NULL,
        0,
        &needed
    );
    if (result == GHOSTTY_SUCCESS && needed == 0) {
        return GHOSTTY_SUCCESS;
    }
    if (result != GHOSTTY_OUT_OF_SPACE || needed == 0) {
        return result;
    }

    uint8_t* data = malloc(needed);
    if (data == NULL) {
        return GHOSTTY_OUT_OF_MEMORY;
    }
    size_t written = 0;
    result = ghostty_mouse_encoder_encode(
        vt->mouse_encoder,
        vt->mouse_event,
        data,
        needed,
        &written
    );
    if (result != GHOSTTY_SUCCESS || written != needed) {
        free(data);
        return result == GHOSTTY_SUCCESS ? GHOSTTY_INVALID_VALUE : result;
    }
    *out_data = data;
    *out_len = written;
    return GHOSTTY_SUCCESS;
}

static int griz_mode(struct GrizVt* vt, GhosttyMode mode, uint8_t* out) {
    GhosttyTerminalModeConfig config = {
        .mode = mode,
        .value = false,
    };
    GhosttyResult result = ghostty_terminal_get(
        vt->terminal,
        GHOSTTY_TERMINAL_DATA_MODE,
        &config
    );
    if (result == GHOSTTY_SUCCESS) {
        *out = config.value ? 1 : 0;
    }
    return result;
}

static int griz_reserve_text(struct GrizVtSnapshot* snapshot, size_t additional) {
    if (additional > SIZE_MAX - snapshot->text_len) {
        return GHOSTTY_OUT_OF_MEMORY;
    }
    size_t needed = snapshot->text_len + additional;
    if (needed <= snapshot->text_cap) {
        return GHOSTTY_SUCCESS;
    }

    size_t capacity = snapshot->text_cap == 0 ? 256 : snapshot->text_cap;
    while (capacity < needed) {
        if (capacity > SIZE_MAX / 2) {
            capacity = needed;
            break;
        }
        capacity *= 2;
    }
    uint8_t* text = realloc(snapshot->text, capacity);
    if (text == NULL) {
        return GHOSTTY_OUT_OF_MEMORY;
    }
    snapshot->text = text;
    snapshot->text_cap = capacity;
    return GHOSTTY_SUCCESS;
}

static int griz_cell_text(
    struct GrizVtSnapshot* snapshot,
    GhosttyRenderStateRowCells cells,
    struct GrizVtCell* out
) {
    GhosttyBuffer buffer = {
        .ptr = NULL,
        .cap = 0,
        .len = 0,
    };
    GhosttyResult result = ghostty_render_state_row_cells_get(
        cells,
        GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8,
        &buffer
    );
    if (result == GHOSTTY_SUCCESS && buffer.len == 0) {
        out->text_offset = snapshot->text_len;
        out->text_len = 0;
        return GHOSTTY_SUCCESS;
    }
    if (result != GHOSTTY_OUT_OF_SPACE || buffer.len == 0) {
        return result;
    }

    result = griz_reserve_text(snapshot, buffer.len);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    size_t offset = snapshot->text_len;
    size_t required = buffer.len;
    buffer.ptr = snapshot->text + offset;
    buffer.cap = required;
    buffer.len = 0;
    result = ghostty_render_state_row_cells_get(
        cells,
        GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8,
        &buffer
    );
    if (result != GHOSTTY_SUCCESS || buffer.len != required) {
        return result == GHOSTTY_SUCCESS ? GHOSTTY_INVALID_VALUE : result;
    }

    snapshot->text_len += buffer.len;
    out->text_offset = offset;
    out->text_len = buffer.len;
    return GHOSTTY_SUCCESS;
}

static void griz_rgb(uint8_t out[3], GhosttyColorRgb color) {
    out[0] = color.r;
    out[1] = color.g;
    out[2] = color.b;
}

static int griz_fill_cell(
    struct GrizVtSnapshot* snapshot,
    GhosttyRenderStateRowCells cells,
    const GhosttyRenderStateColors* colors,
    struct GrizVtCell* out
) {
    GhosttyCell raw = 0;
    GhosttyStyle style;
    GhosttyColorRgb fg;
    GhosttyColorRgb bg;
    GhosttyCellWide wide = GHOSTTY_CELL_WIDE_NARROW;

    GhosttyResult result = ghostty_render_state_row_cells_get(
        cells,
        GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW,
        &raw
    );
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }
    result = ghostty_cell_get(raw, GHOSTTY_CELL_DATA_WIDE, &wide);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }
    result = ghostty_render_state_row_cells_get(
        cells,
        GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
        &style
    );
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    result = ghostty_render_state_row_cells_get(
        cells,
        GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR,
        &fg
    );
    if (result == GHOSTTY_NO_VALUE) {
        fg = colors->foreground;
    } else if (result != GHOSTTY_SUCCESS) {
        return result;
    }
    result = ghostty_render_state_row_cells_get(
        cells,
        GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR,
        &bg
    );
    if (result == GHOSTTY_NO_VALUE) {
        bg = colors->background;
    } else if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    if ((style.flags & GHOSTTY_STYLE_FLAG_INVERSE) != 0) {
        GhosttyColorRgb swap = fg;
        fg = bg;
        bg = swap;
    }

    griz_rgb(out->fg, fg);
    griz_rgb(out->bg, bg);
    out->width = wide == GHOSTTY_CELL_WIDE_WIDE
        ? 2
        : (wide == GHOSTTY_CELL_WIDE_SPACER_TAIL ? 0 : 1);
    out->flags = 0;
    if ((style.flags & GHOSTTY_STYLE_FLAG_BOLD) != 0) {
        out->flags |= GRIZ_CELL_BOLD;
    }
    if ((style.flags & GHOSTTY_STYLE_FLAG_ITALIC) != 0) {
        out->flags |= GRIZ_CELL_ITALIC;
    }
    if (GHOSTTY_STYLE_UNDERLINE(style) != GHOSTTY_SGR_UNDERLINE_NONE) {
        out->flags |= GRIZ_CELL_UNDERLINE;
    }

    return griz_cell_text(snapshot, cells, out);
}

void griz_vt_snapshot_free(struct GrizVtSnapshot* snapshot) {
    if (snapshot == NULL) {
        return;
    }
    free(snapshot->cells);
    free(snapshot->text);
    free(snapshot->title);
    free(snapshot);
}

int griz_vt_snapshot(struct GrizVt* vt, struct GrizVtSnapshot** out) {
    if (vt == NULL || out == NULL) {
        return GHOSTTY_INVALID_VALUE;
    }
    *out = NULL;

    GhosttyResult result = ghostty_render_state_update(vt->render_state, vt->terminal);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    struct GrizVtSnapshot* snapshot = calloc(1, sizeof(*snapshot));
    if (snapshot == NULL) {
        return GHOSTTY_OUT_OF_MEMORY;
    }

    GhosttyRenderStateColors colors = GHOSTTY_INIT_SIZED(GhosttyRenderStateColors);
    GhosttyRenderStateCursor cursor = GHOSTTY_INIT_SIZED(GhosttyRenderStateCursor);
    result = ghostty_render_state_get(
        vt->render_state,
        GHOSTTY_RENDER_STATE_DATA_COLS,
        &snapshot->cols
    );
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    result = ghostty_render_state_get(
        vt->render_state,
        GHOSTTY_RENDER_STATE_DATA_ROWS,
        &snapshot->rows
    );
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    result = ghostty_render_state_get(
        vt->render_state,
        GHOSTTY_RENDER_STATE_DATA_COLORS,
        &colors
    );
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    result = ghostty_render_state_get(
        vt->render_state,
        GHOSTTY_RENDER_STATE_DATA_CURSOR,
        &cursor
    );
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }

    snapshot->cursor_visible = cursor.visible && cursor.viewport_has_value ? 1 : 0;
    if (cursor.viewport_has_value) {
        snapshot->cursor_x = cursor.viewport_x;
        snapshot->cursor_y = cursor.viewport_y;
    }

    result = griz_mode(vt, GHOSTTY_MODE_CURSOR_KEYS, &snapshot->application_cursor);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    result = griz_mode(vt, GHOSTTY_MODE_BRACKETED_PASTE, &snapshot->bracketed_paste);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    uint8_t mouse_mode = 0;
    result = griz_mode(vt, GHOSTTY_MODE_MOUSE_EVENT_X10, &mouse_mode);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    snapshot->mouse_tracking = mouse_mode;
    result = griz_mode(vt, GHOSTTY_MODE_MOUSE_EVENT_NORMAL, &mouse_mode);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    snapshot->mouse_tracking |= mouse_mode;
    result = griz_mode(vt, GHOSTTY_MODE_MOUSE_EVENT_BUTTON, &mouse_mode);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    snapshot->mouse_tracking |= mouse_mode;
    result = griz_mode(vt, GHOSTTY_MODE_MOUSE_EVENT_ANY, &mouse_mode);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    snapshot->mouse_tracking |= mouse_mode;

    GhosttyString title;
    result = ghostty_terminal_get(vt->terminal, GHOSTTY_TERMINAL_DATA_TITLE, &title);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    if (title.len > 0) {
        snapshot->title = malloc(title.len);
        if (snapshot->title == NULL) {
            result = GHOSTTY_OUT_OF_MEMORY;
            goto fail;
        }
        memcpy(snapshot->title, title.ptr, title.len);
        snapshot->title_len = title.len;
    }

    snapshot->cells_len = (size_t)snapshot->cols * (size_t)snapshot->rows;
    snapshot->cells = calloc(snapshot->cells_len, sizeof(*snapshot->cells));
    if (snapshot->cells == NULL && snapshot->cells_len != 0) {
        result = GHOSTTY_OUT_OF_MEMORY;
        goto fail;
    }
    for (size_t i = 0; i < snapshot->cells_len; i++) {
        snapshot->cells[i].width = 1;
        griz_rgb(snapshot->cells[i].fg, colors.foreground);
        griz_rgb(snapshot->cells[i].bg, colors.background);
    }

    result = ghostty_render_state_get(
        vt->render_state,
        GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
        &vt->row_iterator
    );
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }

    while (ghostty_render_state_row_iterator_next(vt->row_iterator)) {
        int32_t y = -1;
        result = ghostty_render_state_row_get(
            vt->row_iterator,
            GHOSTTY_RENDER_STATE_ROW_DATA_VIEWPORT_Y,
            &y
        );
        if (result != GHOSTTY_SUCCESS) {
            goto fail;
        }
        if (y < 0 || y >= snapshot->rows) {
            continue;
        }
        result = ghostty_render_state_row_get(
            vt->row_iterator,
            GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
            &vt->row_cells
        );
        if (result != GHOSTTY_SUCCESS) {
            goto fail;
        }

        uint16_t x = 0;
        while (ghostty_render_state_row_cells_next(vt->row_cells)) {
            if (x >= snapshot->cols) {
                result = GHOSTTY_INVALID_VALUE;
                goto fail;
            }
            struct GrizVtCell* cell = &snapshot->cells[(size_t)y * snapshot->cols + x];
            result = griz_fill_cell(snapshot, vt->row_cells, &colors, cell);
            if (result != GHOSTTY_SUCCESS) {
                goto fail;
            }
            x++;
        }
        if (x != snapshot->cols) {
            result = GHOSTTY_INVALID_VALUE;
            goto fail;
        }
    }

    result = ghostty_render_state_clean(vt->render_state);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    *out = snapshot;
    return GHOSTTY_SUCCESS;

fail:
    griz_vt_snapshot_free(snapshot);
    return result;
}

void griz_vt_take_responses(struct GrizVt* vt, uint8_t** data, size_t* len) {
    if (data == NULL || len == NULL) {
        return;
    }
    *data = NULL;
    *len = 0;
    if (vt == NULL) {
        return;
    }

    *data = vt->responses;
    *len = vt->responses_len;
    vt->responses = NULL;
    vt->responses_len = 0;
    vt->responses_cap = 0;
}

void griz_vt_bytes_free(uint8_t* data) {
    free(data);
}
