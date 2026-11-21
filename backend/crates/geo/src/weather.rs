//! Current weather on pins, from an Open-Meteo compatible API when
//! `TOKULY_WEATHER_URL` is set.
//!
//! Weather is cached in Valkey per cell of a tenth of a degree (about 10 km).
//! Nothing on the hot path waits for the weather service: a missing cell is
//! fetched in the background and shows up on the next push.
use std::time::Duration;

use tokuly_core::prelude::*;

use crate::{Geo, math::Point, visibility::Weather};

const FRESH_FOR: Duration = Duration::from_secs(30 * 60);
/// One fetch per cell at a time, and no hammering a failing service.
const FETCH_EVERY: Duration = Duration::from_secs(5 * 60);

fn cell(point: Point) -> (i32, i32) {
    ((point.0 * 10.0).floor() as i32, (point.1 * 10.0).floor() as i32)
}

fn cache_key(cell: (i32, i32)) -> String {
    format!("geo:wx:{}:{}", cell.0, cell.1)
}

fn fetch_key(cell: (i32, i32)) -> String {
    format!("geo:wx:fetch:{}:{}", cell.0, cell.1)
}

/// Icon name for a WMO weather code (the codes Open-Meteo reports).
pub fn icon_for(code: i64, is_day: bool) -> &'static str {
    match code {
        0 | 1 if is_day => "clear-day",
        0 | 1 => "clear-night",
        2 if is_day => "partly-cloudy-day",
        2 => "partly-cloudy-night",
        3 => "cloudy",
        45 | 48 => "fog",
        51..=57 => "drizzle",
        61..=67 | 80..=82 => "rain",
        71..=77 | 85 | 86 => "snow",
        95..=99 => "thunderstorm",
        _ => "cloudy",
    }
}

#[derive(serde::Deserialize)]
struct Forecast {
    current: Current,
}

#[derive(serde::Deserialize)]
struct Current {
    temperature_2m: f64,
    #[serde(default)]
    weather_code: i64,
    #[serde(default = "day")]
    is_day: i64,
}

fn day() -> i64 {
    1
}

impl Geo {
    /// Weather at each point from the cache (`None` when unknown, when the
    /// point is `None`, or when no weather service is configured). Cells
    /// missing from the cache are requested in the background.
    pub(crate) async fn weather_at(&self, points: &[Option<Point>]) -> Vec<Option<Weather>> {
        if self.cfg.weather_url.is_empty() || points.iter().all(Option::is_none) {
            return vec![None; points.len()];
        }
        let keys: Vec<String> = points.iter().map(|point| point.map_or_else(|| "geo:wx:none".to_owned(), |point| cache_key(cell(point)))).collect();
        let cached: Vec<Option<Vec<u8>>> = match redis::cmd("MGET").arg(&keys).query_async(&mut self.core.cache.conn()).await {
            Ok(values) => values,
            Err(e) => {
                tracing::debug!(error = %e, "weather cache unavailable");
                return vec![None; points.len()];
            }
        };
        let mut out = Vec::with_capacity(points.len());
        let mut requested = Vec::new();
        for (point, bytes) in points.iter().zip(cached) {
            let weather: Option<Weather> = bytes.and_then(|bytes| serde_json::from_slice(&bytes).ok());
            if let (None, Some(point)) = (&weather, point) {
                if !requested.contains(&cell(*point)) {
                    requested.push(cell(*point));
                    self.request_weather(*point);
                }
            }
            out.push(weather);
        }
        out
    }

    /// Fetches the weather of a cell in the background, unless someone else
    /// did so in the last few minutes.
    fn request_weather(&self, point: Point) {
        let geo = self.clone();
        tokio::spawn(async move {
            let cell = cell(point);
            match geo.core.cache.set_nx(&fetch_key(cell), "1", FETCH_EVERY).await {
                Ok(true) => {}
                _ => return,
            }
            match geo.fetch_weather(point).await {
                Ok(weather) => {
                    if let Err(e) = geo.core.cache.set_json(&cache_key(cell), &weather, Some(FRESH_FOR)).await {
                        tracing::debug!(error = %e, "weather not cached");
                    }
                }
                Err(e) => tracing::debug!(error = %e, "weather lookup failed"),
            }
        });
    }

    async fn fetch_weather(&self, point: Point) -> AppResult<Weather> {
        let url = format!("{}/v1/forecast", self.cfg.weather_url.trim_end_matches('/'));
        let forecast: Forecast = self
            .core
            .http
            .get(url)
            .query(&[("latitude", format!("{:.2}", point.0)), ("longitude", format!("{:.2}", point.1)), ("current", "temperature_2m,weather_code,is_day".to_owned())])
            .timeout(Duration::from_secs(5))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(Weather { temperature_c: forecast.current.temperature_2m, icon: icon_for(forecast.current.weather_code, forecast.current.is_day != 0).to_owned() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_are_a_tenth_of_a_degree() {
        assert_eq!(cell((35.6595, 139.7005)), (356, 1397));
        assert_eq!(cell((-33.86, 151.21)), (-339, 1512));
        assert_eq!(cell((35.61, 139.79)), cell((35.69, 139.71)));
        assert_ne!(cell((35.59, 139.7)), cell((35.61, 139.7)));
        assert_eq!(cache_key((356, 1397)), "geo:wx:356:1397");
    }

    #[test]
    fn weather_codes_map_to_icons() {
        assert_eq!(icon_for(0, true), "clear-day");
        assert_eq!(icon_for(1, false), "clear-night");
        assert_eq!(icon_for(2, true), "partly-cloudy-day");
        assert_eq!(icon_for(3, false), "cloudy");
        assert_eq!(icon_for(48, true), "fog");
        assert_eq!(icon_for(55, true), "drizzle");
        assert_eq!(icon_for(63, true), "rain");
        assert_eq!(icon_for(81, false), "rain");
        assert_eq!(icon_for(75, true), "snow");
        assert_eq!(icon_for(96, true), "thunderstorm");
        assert_eq!(icon_for(1234, true), "cloudy");
    }

    #[test]
    fn open_meteo_responses_parse() {
        let body = r#"{"latitude":35.7,"longitude":139.69,"current_units":{"temperature_2m":"°C"},
                       "current":{"time":"2026-11-20T12:00","interval":900,"temperature_2m":14.3,"weather_code":61,"is_day":1}}"#;
        let forecast: Forecast = serde_json::from_str(body).unwrap();
        assert_eq!((forecast.current.temperature_2m, forecast.current.weather_code, forecast.current.is_day), (14.3, 61, 1));
        // Older servers without the extra fields still give a temperature.
        let minimal: Forecast = serde_json::from_str(r#"{"current":{"temperature_2m":-2.0}}"#).unwrap();
        assert_eq!((minimal.current.weather_code, minimal.current.is_day), (0, 1));
    }
}
