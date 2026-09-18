/**
 * lares-hud — literal amber HUD for the Lares cleaning assistant.
 *
 * Polls GET /v1/hud from the Lares server every 3 seconds and renders
 * the mission state on a 64x128 SH1107 I2C OLED behind a lens combiner.
 *
 * Hardware: ESP32-S3 + 1.3" 64x128 yellow OLED (SH1107, I2C addr 0x3C)
 * Libraries: u8g2, ArduinoJson
 */

#include <Arduino.h>
#include <WiFi.h>
#include <HTTPClient.h>
#include <ArduinoJson.h>
#include <U8g2lib.h>
#include <Wire.h>

// ── Configuration ────────────────────────────────────────────────────
// Set these before flashing, or override at runtime via serial commands.
#ifndef WIFI_SSID
#define WIFI_SSID "Lares"
#endif
#ifndef WIFI_PASS
#define WIFI_PASS "l4r3s_n3t"
#endif
#ifndef SERVER_HOST
#define SERVER_HOST "100.111.244.0"
#endif
#ifndef SERVER_PORT
#define SERVER_PORT 8787
#endif

// Poll interval in milliseconds.
static const unsigned long POLL_MS = 3000;

// ── OLED ─────────────────────────────────────────────────────────────
// 64x128 yellow SH1107, I2C, SDA=GPIO21, SCL=GPIO22 (ESP32-S3 defaults).
// Rotate 180 if the combiner inverts the image.
U8G2_SH1107_64X128_F_HW_I2C oled(U8G2_R2, /*clock=*/22, /*data=*/21);

// ── HUD payload (parsed from /v1/hud) ────────────────────────────────
struct HudPayload {
    String room;
    int targets;
    String nextTitle;
    int nextDays;
    // Up to 3 tasks for the mini ticker.
    struct Task {
        String title;
        int days;
    } tasks[3];
    int taskCount;
    long updatedAt;
};

// ── Helpers ──────────────────────────────────────────────────────────

/** Connect to WiFi with a bounded retry. */
void wifiConnect() {
    Serial.printf("WiFi: connecting to %s\n", WIFI_SSID);
    WiFi.begin(WIFI_SSID, WIFI_PASS);
    unsigned long start = millis();
    while (WiFi.status() != WL_CONNECTED && millis() - start < 15000) {
        delay(250);
    }
    if (WiFi.status() == WL_CONNECTED) {
        Serial.printf("WiFi: connected, IP %s\n", WiFi.localIP().toString().c_str());
    } else {
        Serial.println("WiFi: connection timed out, will retry next poll");
    }
}

/** Poll the Lares server for the HUD payload. Returns true on success. */
bool fetchHud(HudPayload &hud) {
    if (WiFi.status() != WL_CONNECTED) {
        wifiConnect();
        if (WiFi.status() != WL_CONNECTED) return false;
    }

    String url = "http://" + String(SERVER_HOST) + ":" + String(SERVER_PORT) + "/v1/hud";
    HTTPClient http;
    http.begin(url);
    http.setTimeout(4000);
    int code = http.GET();
    if (code != 200) {
        Serial.printf("HUD HTTP %d\n", code);
        http.end();
        return false;
    }

    JsonDocument doc;
    DeserializationError err = deserializeJson(doc, http.getString());
    http.end();
    if (err) {
        Serial.printf("HUD JSON: %s\n", err.c_str());
        return false;
    }

    hud.room = doc["room"].as<String>();
    hud.targets = doc["targets"].as<int>();
    hud.updatedAt = doc["updated_at"].as<long>();
    hud.taskCount = 0;
    if (doc["next_task"].is<JsonObject>()) {
        hud.nextTitle = doc["next_task"]["title"].as<String>();
        hud.nextDays = doc["next_task"]["days"].as<int>();
    } else {
        hud.nextTitle = "";
        hud.nextDays = 0;
    }
    JsonArray tasks = doc["tasks"].as<JsonArray>();
    for (int i = 0; i < 3 && i < (int)tasks.size(); i++) {
        hud.tasks[i].title = tasks[i]["title"].as<String>();
        hud.tasks[i].days = tasks[i]["days"].as<int>();
        hud.taskCount++;
    }
    return true;
}

// ── Drawing ──────────────────────────────────────────────────────────

/** Truncate a string to fit within `maxPx` pixels at the current font. */
String fitWidth(const String &text, int maxPx) {
    int w = u8g2.getUTF8Width(text.c_str());
    if (w <= maxPx) return text;
    String out = text;
    while (out.length() > 0 && u8g2.getUTF8Width((out + "...").c_str()) > maxPx) {
        out.remove(out.length() - 1);
    }
    return out + "..";
}

/** Render the full HUD frame. */
void renderHud(const HudPayload &hud) {
    oled.clearBuffer();

    // Hairline border — the frame.
    oled.drawFrame(0, 0, 64, 128);

    // ── Top block: room + target count ──
    // Thin horizontal separator.
    oled.drawHLine(2, 14, 60);

    oled.setFont(u8g2_font_5x7_tf);
    oled.setCursor(3, 10);
    oled.print(fitWidth(hud.room.toUpperCase(), 40).c_str());

    oled.setCursor(45, 10);
    oled.print("T");
    oled.print(hud.targets);

    // ── Middle block: next task ──
    oled.drawHLine(2, 42, 60);

    if (hud.nextTitle.length() > 0) {
        oled.setCursor(3, 24);
        oled.setFont(u8g2_font_5x7_tf);
        oled.print("NEXT");

        // Title wrapped across two lines (max ~12 chars per line at 5px).
        String line1 = fitWidth(hud.nextTitle, 56);
        oled.setCursor(3, 34);
        oled.print(line1.c_str());

        // Days badge right-aligned.
        String days = String(abs(hud.nextDays)) + "D";
        if (hud.nextDays < 0) days = "-" + days;
        int dw = u8g2.getUTF8Width(days.c_str());
        oled.setCursor(61 - dw, 34);
        oled.print(days.c_str());
    } else {
        oled.setFont(u8g2_font_5x7_tf);
        oled.setCursor(3, 30);
        oled.print("NO TASKS");
    }

    // ── Bottom block: mini ticker ──
    oled.drawHLine(2, 72, 60);

    oled.setFont(u8g2_font_4x6_tf);
    for (int i = 0; i < hud.taskCount; i++) {
        int y = 82 + i * 10;
        if (y > 120) break;
        oled.setCursor(3, y);
        oled.print(fitWidth(hud.tasks[i].title, 42).c_str());

        String days = String(abs(hud.tasks[i].days)) + "D";
        if (hud.tasks[i].days < 0) days = "-" + days;
        int dw = u8g2.getUTF8Width(days.c_str());
        oled.setCursor(61 - dw, y);
        oled.print(days.c_str());
    }

    // ── Blinking cursor (lower left) ──
    if ((millis() / 500) % 2 == 0) {
        oled.drawBox(3, 120, 6, 4);
    }

    oled.sendBuffer();
}

// ── Setup / loop ─────────────────────────────────────────────────────

void setup() {
    Serial.begin(115200);
    delay(200);

    oled.begin();
    oled.setBusClock(400000);
    oled.clearBuffer();
    oled.setFont(u8g2_font_5x7_tf);
    oled.setCursor(3, 30);
    oled.print("lares-hud");
    oled.setCursor(3, 42);
    oled.print("booting...");
    oled.sendBuffer();

    wifiConnect();
}

void loop() {
    static unsigned long lastPoll = 0;
    if (millis() - lastPoll < POLL_MS) return;
    lastPoll = millis();

    HudPayload hud;
    if (fetchHud(hud)) {
        renderHud(hud);
    } else {
        // Draw a small "..." blink so the display isn't blank.
        oled.clearBuffer();
        oled.drawFrame(0, 0, 64, 128);
        oled.setFont(u8g2_font_5x7_tf);
        oled.setCursor(3, 30);
        oled.print("connecting...");
        if ((millis() / 500) % 2 == 0) {
            oled.drawBox(3, 120, 6, 4);
        }
        oled.sendBuffer();
    }
}
