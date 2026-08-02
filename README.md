# HydrantenFinder

Ein in **Rust** geschriebenes CLI-Tool, das den aktuellen Windows-Standort nutzt, um in der Nähe befindliche Feuerhydranten (über die Overpass-API / OpenStreetMap) zu ermitteln. Das Programm berechnet zu Fuß erreichbare Routen, bewertet die Hydranten anhand spezifischer Eigenschaften (wie Typ, Durchmesser und Kupplungen) über ein Scoring-Modell, speichert Ergebnisse in einer lokalen MySQL-Datenbank zum Caching und zeigt die Route visuell auf einer statischen Karte in einem Fenster an.

---

## 🛠️ Tech Stack & Verwendete Libraries

- **Sprache:** Rust
- **Asynchroner Runtime & Task-Management:** `tokio`
- **HTTP-Client & APIs:** `reqwest`, `serde` / `serde_json`
- **Geodaten & Routing:** `geo`, `polyline`, Overpass API, OpenRouteService (ORS), OSRM
- **Windows Integration:** `windows` (Geolocation API), `windows-sys`
- **Datenbank:** `mysql` (MySQL / MariaDB Caching-Layer)
- **GUI & Darstellung:** `minifb` (Fensterdarstellung), `image` (Bildverarbeitung)
- **CLI & UX:** `indicatif` (Progress Bars & Spinner), `anyhow` (Error Handling), `dotenvy`

---

## ⚙️ Funktionsweise & Architektur

Das Projekt ist modular aufgebaut und teilt sich in folgende Kernbereiche auf:

1. **Standortbestimmung (`utils`):** Ermittelt via Windows Geolocation API die aktuellen Koordinaten des Geräts (mit Fallback- und Timeout-Logik).
2. **Datenabfrage & Routing (`api`):**
    - Ruft via Overpass API Feuerhydranten (`emergency=fire_hydrant`) im Umkreis ab (inklusive automatischer Radius-Vergrößerung und Retry-Mechanismus bei Netzwerkfehlern).
    - Berechnet Fußwege über OSRM mit automatischem Fallback auf OpenRouteService (ORS) sowie Luftlinien-Fallback.
    - Parallele Routenberechnung für mehrere Kandidaten mittels Tokio-Tasks.
3. **Scoring-System (`models`):** Bewertet gefundene Hydranten anhand eines gewichteten Scores (kombiniert Fußwegdistanz, Rohrdurchmesser, Bauart wie Über-/Unterflur, Position und Anzahl der Kupplungen).
4. **Caching-Layer (`database`):** Speichert Hydranten-Stammdaten und berechnete Routen per Geohash in einer lokalen MySQL-Datenbank, um wiederholte API-Abfragen zu minimieren.
5. **Visualisierung:** Lädt eine statische Routenkarte von Stadia Maps herunter, skaliert und rendert sie lokal in einem `minifb`-Fenster.

---

## 📋 Voraussetzungen & Konfiguration

Da das Projekt als Übungsprojekt und primär für Windows konzipiert wurde, gelten folgende Voraussetzungen:

- **Betriebssystem:** Windows (aufgrund der nativen Nutzung der Windows Geolocation API und Windows-Konsoleneinstellungen).
- **Datenbank:** Eine lokale MySQL- oder MariaDB-Instanz (`mysql://root@localhost:3306/hydranten_cache`).
- **Umgebungsvariablen:** Eine `.env`-Datei im Stammverzeichnis mit folgenden API-Schlüsseln:
  ```env
  ORS_API_KEY=dein_openrouteservice_api_key
  STADIAMAPS_API_KEY=dein_stadiamaps_api_key
  ```

---

## 🚀 Installation & Start

1. Repository klonen:
   ```bash
   git clone https://github.com/dein-benutzername/hydranten-finder.git
   cd hydranten-finder
   ```
2. `.env`-Datei anlegen und API-Schlüssel eintragen.
3. Lokale MySQL-Datenbank starten und sicherstellen, dass die Datenbank `hydranten_cache` existiert.
4. Projekt starten:
   ```bash
   cargo run --release
   ```

---

## 💡 Hinweis zum Projektkontext

*Dieses Projekt ist im Rahmen meiner Einarbeitung und zum praktischen Üben in Rust entstanden. Der Fokus lag hierbei primär darauf, verschiedene Technologien (Asynchrones Rust mit Tokio, native Windows-APIs, REST-Schnittstellen, Datenbankanbindung und Bildverarbeitung) miteinander zu verknüpfen und auszuprobieren. Daher liegt der Schwerpunkt hier klar auf der praktischen Umsetzung und dem Ausprobieren neuer Konzepte statt auf perfekten Enterprise-Standards.*
