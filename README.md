# HydrantenFinder

Eine in **Rust** geschriebene Desktop-Anwendung (`eframe` / `egui`), die den aktuellen Windows-Standort nutzt, um in der Nähe befindliche Feuerhydranten (über die Overpass-API / OpenStreetMap) zu ermitteln. Das Programm berechnet parallel zu Fuß erreichbare Routen über OSRM (mit OpenRouteService-Fallback), sortiert die Hydranten nach Routendistanz, speichert Ergebnisse zur Zwischenspeicherung in einer lokalen MySQL-Datenbank und stellt die Hydranten samt Routen interaktiv auf einer OpenStreetMap-Karte dar.

---

## 🛠️ Tech Stack & Verwendete Libraries

- **Sprache:** Rust
- **Asynchrone Runtime & Task-Management:** `tokio`
- **GUI & Kartendarstellung:** `eframe` / `egui` (Desktop-Oberfläche & Seitenleiste), `egui_map_view` (interaktive OpenStreetMap-Karte & eigener Overlay-`Layer`)
- **HTTP-Client & Serialisierung:** `reqwest`, `serde`, `serde_json`
- **Geodaten & Routing:** `geo`, `geo-types`, `polyline`, `geohash`, Overpass API (mit Mirror-Failover), OSRM, OpenRouteService (ORS)
- **Windows-Integration:** `windows` (Windows Geolocation API), `windows-future`
- **Datenbank:** `mysql` (MySQL / MariaDB Caching-Layer), `chrono`
- **Konsolen-Ausgabe & Hilfsbibliotheken:** `indicatif` (Progress-Spinner im Terminal), `anyhow` (Error Handling), `dotenvy`, `ctrlc`, `rand`

---

## ⚙️ Funktionsweise & Architektur

Das Projekt ist modular aufgebaut und teilt sich in folgende Kernbereiche auf:

1. **Einstiegspunkt & Ablaufsteuerung (`main`):** Initialisiert den geteilten, thread-sicheren Anwendungszustand (`AppState`), startet die Standort- und Hydrantensuche asynchron in einem Tokio-Hintergrundtask und öffnet das maximierte GUI-Fenster auf dem Main-Thread.
2. **Standortbestimmung & Initialisierung (`utils`):**
    - Ermittelt via Windows Geolocation API die aktuellen Koordinaten des Geräts (mit hoher Genauigkeit, maximalem Cache-Alter von 5 Minuten und 5-Sekunden-Timeout).
    - Berechnet einen geeigneten Karten-Zoomlevel anhand der Haversine-Distanz.
    - Lädt Umgebungsvariablen, initialisiert Datenbank sowie HTTP-Client und registriert einen `Ctrl+C`-Handler, der laufende Datenbank-Schreibvorgänge vor dem Beenden sauber abschließt.
3. **Datenabfrage & Routing (`api`):**
    - Ruft via Overpass API Feuerhydranten (`emergency=fire_hydrant`) im Umkreis ab (Startradius 500 m mit schrittweiser Verdopplung bis max. 1.500 m bei weniger als 5 Treffern) – inklusive automatischem Failover zwischen mehreren Overpass-Mirrors und Retry-Mechanismus (Exponential Backoff + Jitter).
    - Wählt die nach Luftlinie 5 nächstgelegenen Kandidaten aus und berechnet deren Fußwege parallel in Tokio-Tasks (mit 5-Sekunden-Gesamt-Timeout).
    - Nutzt primär OSRM für das Fußgänger-Routing mit automatischem Fallback auf OpenRouteService (ORS) sowie Luftlinien-Fallback bei unmittelbarer Nähe ($\le 5\text{ m}$) und sortiert die Hydranten aufsteigend nach Routenlänge.
4. **Datenmodelle (`models`):** Definiert Strukturen und Enums für API-Antworten (Overpass, OSRM, ORS), Koordinaten, Hydranten inklusive OSM-Metadaten (`fire_hydrant:type`, `position`, `diameter`, `couplings`), Routendetails sowie Cache-Einträge.
5. **Caching-Layer (`database`):** Verwaltet einen globalen MySQL-Verbindungspool, erstellt die benötigten Tabellen (`hydrants` und `route_cache`) automatisch und speichert Hydranten-Stammdaten sowie berechnete Routen transaktional per 8-stelligem Geohash (Gültigkeit: 2 Wochen), um wiederholte API-Abfragen zu vermeiden.
6. **Interaktive GUI (`gui`):**
    - Rendert eine OpenStreetMap-Ansicht mit eigenem `DrawLayer` für den Benutzerstandort, farblich unterscheidbare Hydrantenmarker und die hervorgehobene Route als Polyline.
    - Zeigt in einer rechten Seitenleiste Details zu jedem Hydranten an (Routendistanz, Bauart wie Über-/Unterflur-, Rohr- oder Wandhydrant, Rohrdurchmesser, Position wie Gehweg/Fahrbahn/Grünfläche/Parkplatz und Anzahl der Kupplungen).
    - Ermöglicht den Wechsel zwischen den angezeigten Routen per Klick auf einen Hydrantenmarker auf der Karte oder einen Eintrag in der Seitenleiste.

---

## 📋 Voraussetzungen & Konfiguration

Da das Projekt als Übungsprojekt und primär für Windows konzipiert wurde, gelten folgende Voraussetzungen:

- **Betriebssystem:** Windows (aufgrund der nativen Nutzung der Windows Geolocation API).
- **Datenbank:** Eine laufende lokale MySQL- oder MariaDB-Instanz unter `mysql://root@localhost:3306/hydranten_cache` (die Datenbank `hydranten_cache` muss existieren; die Tabellen werden beim Start automatisch angelegt).
- **Umgebungsvariablen:** Eine `.env`-Datei im Stammverzeichnis mit deinem API-Schlüssel für den OpenRouteService-Fallback (wird benötigt, falls OSRM nicht erreichbar ist oder keine Route liefert):
  ```env
  ORS_API_KEY=dein_openrouteservice_api_key
  ```
---

## 💡 Hinweis zum Projektkontext

*Dieses Projekt ist im Rahmen meiner Einarbeitung und zum praktischen Üben in Rust entstanden. Der Fokus lag hierbei primär darauf, verschiedene Technologien (asynchrones Rust mit Tokio, native Windows-APIs, REST-Schnittstellen, Datenbankanbindung und interaktive GUI-/Kartendarstellung) miteinander zu verknüpfen und auszuprobieren. Daher liegt der Schwerpunkt hier klar auf der praktischen Umsetzung und dem Ausprobieren neuer Konzepte statt auf perfekten Enterprise-Standards.*