//! The optional external services (a Photon geocoder, a Valhalla routing
//! engine, an Open-Meteo weather API) against a local stand-in that speaks
//! just enough of each, and what happens when they are down.
mod common;

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use common::*;
use serde_json::json;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
};
use tokuly_geo::{GeoConfig, math};
use tokuly_proto::geo::{destination::Target, *};

const SHIBUYA: Point = (35.6595, 139.7005);
const TOKYO_STATION: Point = (35.681236, 139.767125);
/// A café the stand-in geocoder knows.
const CAFE: Point = (35.6612, 139.6981);

/// One request the stand-in received.
#[derive(Debug, Clone)]
struct Received {
    method: String,
    path: String,
    query: HashMap<String, String>,
    body: serde_json::Value,
}

/// A local HTTP server answering like Photon (`/api`, `/reverse`), Valhalla
/// (`/route`) and Open-Meteo (`/v1/forecast`).
#[derive(Clone)]
struct StandIn {
    url: String,
    received: Arc<Mutex<Vec<Received>>>,
    down: Arc<AtomicBool>,
}

impl StandIn {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free local port");
        let url = format!("http://{}", listener.local_addr().expect("local address"));
        let stand_in = Self { url, received: Arc::default(), down: Arc::default() };
        let server = stand_in.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(server.clone().serve(socket));
            }
        });
        stand_in
    }

    /// Answers the one request of a connection.
    async fn serve(self, mut socket: TcpStream) {
        let Some(request) = read_request(&mut socket).await else { return };
        let (status, body) = if self.down.load(Ordering::Relaxed) {
            (500, json!({ "error": "down for maintenance" }))
        } else {
            answer(&request)
        };
        self.received.lock().unwrap().push(request);
        let body = body.to_string();
        let head = format!(
            "HTTP/1.1 {status} Stand-In\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        let _ = socket.write_all(head.as_bytes()).await;
        let _ = socket.write_all(body.as_bytes()).await;
        let _ = socket.shutdown().await;
    }

    /// The requests received on `path` so far.
    fn requests(&self, path: &str) -> Vec<Received> {
        self.received.lock().unwrap().iter().filter(|request| request.path == path).cloned().collect()
    }

    fn set_down(&self, down: bool) {
        self.down.store(down, Ordering::Relaxed);
    }
}

/// Reads one HTTP/1.1 request: the head, then as many body bytes as it announces.
async fn read_request(socket: &mut TcpStream) -> Option<Received> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut request_line = lines.next()?.split_whitespace();
    let (method, target) = (request_line.next()?.to_owned(), request_line.next()?.to_owned());
    let length: usize = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse().ok())
        .unwrap_or(0);
    while buffer.len() < head_end + length {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let url = reqwest::Url::parse(&format!("http://stand-in{target}")).ok()?;
    Some(Received {
        method,
        path: url.path().to_owned(),
        query: url.query_pairs().map(|(key, value)| (key.into_owned(), value.into_owned())).collect(),
        body: serde_json::from_slice(&buffer[head_end..]).unwrap_or(serde_json::Value::Null),
    })
}

fn answer(request: &Received) -> (u16, serde_json::Value) {
    match request.path.as_str() {
        "/api" | "/reverse" => (200, photon_answer(request.query.get("lang").is_some_and(|lang| lang == "en"))),
        // The stand-in has no cycle paths.
        "/route" if request.body["costing"] == "bicycle" => {
            (400, json!({ "error_code": 442, "error": "No path could be found for input", "status_code": 400 }))
        }
        "/route" => (200, valhalla_answer(&request.body)),
        "/v1/forecast" => (
            200,
            json!({
                "latitude": 35.7, "longitude": 139.7, "current_units": { "temperature_2m": "°C" },
                "current": { "time": "2026-11-22T12:00", "interval": 900, "temperature_2m": 14.3, "weather_code": 61, "is_day": 1 },
            }),
        ),
        _ => (404, json!({ "error": "no such endpoint" })),
    }
}

/// One café in Shibuya, as Photon describes it (GeoJSON).
fn photon_answer(english: bool) -> serde_json::Value {
    let properties = if english {
        json!({ "name": "Mocha Coffee", "city": "Shibuya", "state": "Tokyo", "country": "Japan", "countrycode": "JP", "osm_key": "amenity" })
    } else {
        json!({ "name": "モカ珈琲店", "city": "渋谷区", "state": "東京都", "country": "日本", "countrycode": "JP", "osm_key": "amenity" })
    };
    json!({
        "type": "FeatureCollection",
        "features": [{ "type": "Feature", "geometry": { "type": "Point", "coordinates": [CAFE.1, CAFE.0] }, "properties": properties }],
    })
}

/// A route from the first to the last requested location with a detour in
/// between, as Valhalla describes it (shape at precision 6, kilometers).
fn valhalla_answer(request: &serde_json::Value) -> serde_json::Value {
    let location = |index: usize| {
        (request["locations"][index]["lat"].as_f64().unwrap(), request["locations"][index]["lon"].as_f64().unwrap())
    };
    let (from, to) = (location(0), location(1));
    let detour = ((from.0 + to.0) / 2.0 + 0.004, (from.1 + to.1) / 2.0);
    let shape =
        polyline::encode_coordinates([from, detour, to].map(|(lat, lng)| geo::Coord { x: lng, y: lat }), 6).unwrap();
    json!({
        "trip": {
            "status": 0, "units": "kilometers", "language": request["language"],
            "summary": { "length": 7.5, "time": 930.4 },
            "legs": [{
                "shape": shape,
                "summary": { "length": 7.5, "time": 930.4 },
                "maneuvers": [
                    { "type": 1, "instruction": "北東に進む。", "length": 7.4, "time": 900.0 },
                    { "type": 4, "instruction": "目的地に到着しました。", "length": 0.1, "time": 30.4 },
                ],
            }],
        },
    })
}

fn to_point(point: Point) -> Destination {
    Destination { target: Some(Target::Position(Box::new(lat_lng(point)))), ..Default::default() }
}

async fn search(ctx: &Ctx, user: &tokuly_testkit::TestUser, query: &str, near: Point) -> Vec<PlaceSearchResult> {
    let request = SearchPlacesRequest { query: query.to_owned(), near: lat_lng(near).into(), ..Default::default() };
    ctx.places::<_, SearchPlacesResponse>("SearchPlaces", user, &request).await.expect("search places").results
}

async fn reverse(ctx: &Ctx, user: &tokuly_testkit::TestUser, position: Point, language: &str) -> GeocodeResult {
    let request = ReverseGeocodeRequest {
        position: lat_lng(position).into(),
        language: language.to_owned(),
        ..Default::default()
    };
    ctx.places::<_, ReverseGeocodeResponse>("ReverseGeocode", user, &request)
        .await
        .expect("reverse geocode")
        .result
        .into_option()
        .expect("a result")
}

async fn directions(ctx: &Ctx, user: &tokuly_testkit::TestUser, mode: TravelMode) -> GetDirectionsResponse {
    let request = GetDirectionsRequest {
        origin: lat_lng(SHIBUYA).into(),
        destination: to_point(TOKYO_STATION).into(),
        mode: mode.into(),
        ..Default::default()
    };
    ctx.trips("GetDirections", user, &request).await.expect("directions")
}

#[tokio::test]
async fn an_external_geocoder_answers_search_and_reverse_geocoding() {
    let photon = StandIn::start().await;
    // A trailing slash in the setting is harmless.
    let ctx = start_with(GeoConfig { geocoder_url: format!("{}/", photon.url), ..Default::default() }).await;
    let aoi = ctx.app.user("aoi").await;

    // Search: geocoder hits come after Tokuly places (none here), with distance from the bias point.
    let found = search(&ctx, &aoi, "モカ", SHIBUYA).await;
    assert_eq!(found.len(), 1);
    assert!(found[0].place.as_option().is_none(), "a raw geocoder hit, not a Tokuly place");
    assert_eq!((found[0].name.as_str(), found[0].address.as_str()), ("モカ珈琲店", "渋谷区, 東京都, 日本"));
    assert!(at(point(found[0].position.as_option()), CAFE));
    assert!((found[0].distance_meters.unwrap() - distance(SHIBUYA, CAFE)).abs() < 1.0);
    let asked = photon.requests("/api");
    assert_eq!(asked.len(), 1);
    assert_eq!((asked[0].method.as_str(), asked[0].query["q"].as_str()), ("GET", "モカ"));
    assert_eq!((asked[0].query["lat"].as_str(), asked[0].query["lon"].as_str()), ("35.6595", "139.7005"));
    assert!(asked[0].query["limit"].parse::<i32>().unwrap() > 0);
    assert!(!asked[0].query.contains_key("lang"), "Japanese accounts get the local names");

    // Reverse geocoding: names from the geocoder, the time zone from the bundled data.
    let here = reverse(&ctx, &aoi, CAFE, "").await;
    assert_eq!(
        (here.name.as_str(), here.locality.as_str(), here.region.as_str(), here.country.as_str()),
        ("モカ珈琲店", "渋谷区", "東京都", "日本")
    );
    assert_eq!((here.country_code.as_str(), here.time_zone.as_str()), ("JP", "Asia/Tokyo"));
    let english = reverse(&ctx, &aoi, CAFE, "en-US").await;
    assert_eq!(
        (english.name.as_str(), english.locality.as_str(), english.country.as_str()),
        ("Mocha Coffee", "Shibuya", "Japan")
    );
    let asked = photon.requests("/reverse");
    assert_eq!(asked.len(), 2);
    assert_eq!((asked[0].query["lat"].as_str(), asked[0].query["lon"].as_str()), ("35.6612", "139.6981"));
    assert_eq!(asked[1].query["lang"], "en");
}

#[tokio::test]
async fn a_routing_engine_gives_real_routes() {
    let valhalla = StandIn::start().await;
    let ctx = start_with(GeoConfig { routing_url: valhalla.url.clone(), ..Default::default() }).await;
    let aoi = ctx.app.user("aoi").await;

    let drive = directions(&ctx, &aoi, TravelMode::TRAVEL_MODE_DRIVE).await;
    assert_eq!(drive.provider, "valhalla");
    assert_eq!((drive.distance_meters, drive.duration_seconds), (7500.0, 930));
    // The engine's shape (precision 6) comes back as a standard polyline (precision 5).
    let line = math::decode_polyline(&drive.polyline, 5);
    assert_eq!(line.len(), 3);
    assert!(distance(line[0], SHIBUYA) < 2.0 && distance(line[2], TOKYO_STATION) < 2.0);
    assert!(distance(line[1], SHIBUYA) > 1000.0 && distance(line[1], TOKYO_STATION) > 1000.0, "the detour is kept");
    let steps: Vec<(&str, f64, i32)> = drive
        .steps
        .iter()
        .map(|step| (step.instruction.as_str(), step.distance_meters.round(), step.duration_seconds))
        .collect();
    assert_eq!(steps, vec![("北東に進む。", 7400.0, 900), ("目的地に到着しました。", 100.0, 30)]);
    let asked = valhalla.requests("/route");
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].method, "POST");
    assert_eq!(asked[0].body["costing"], "auto");
    assert_eq!(asked[0].body["units"], "kilometers");
    assert_eq!(asked[0].body["language"], "ja-JP");
    assert_eq!(
        asked[0].body["locations"],
        json!([{ "lat": SHIBUYA.0, "lon": SHIBUYA.1 }, { "lat": TOKYO_STATION.0, "lon": TOKYO_STATION.1 }])
    );
    assert_eq!(directions(&ctx, &aoi, TravelMode::TRAVEL_MODE_WALK).await.provider, "valhalla");
    assert_eq!(valhalla.requests("/route")[1].body["costing"], "pedestrian");

    // "No route" is an answer, not an outage: the estimate takes over.
    let bike = directions(&ctx, &aoi, TravelMode::TRAVEL_MODE_BICYCLE).await;
    assert_eq!(bike.provider, "estimate");
    assert!((6300.0..6500.0).contains(&bike.distance_meters), "{}", bike.distance_meters);
    assert_eq!(bike.steps.len(), 1);
}

#[tokio::test]
async fn weather_appears_on_pins_once_it_was_fetched() {
    let open_meteo = StandIn::start().await;
    let ctx = start_with(GeoConfig { weather_url: open_meteo.url.clone(), ..Default::default() }).await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    // The middle of a weather cell (a tenth of a degree) no other run has cached.
    let anywhere = somewhere();
    let here = ((anywhere.0 * 10.0).floor() / 10.0 + 0.05, (anywhere.1 * 10.0).floor() / 10.0 + 0.05);
    ctx.at(&aoi, here).await;

    // Nothing waits for the weather service: the pin gets its weather as soon as the cell is cached.
    let mut pin = ctx.seen_by(&ren, &aoi).await.expect("a pin");
    for _ in 0..50 {
        if pin.temperature_celsius.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        pin = ctx.seen_by(&ren, &aoi).await.expect("a pin");
    }
    assert_eq!((pin.temperature_celsius, pin.weather_icon.as_str()), (Some(14.3), "rain"));
    assert_at!(pin_point(&pin), here);

    // One lookup per cell, however often the map is opened and whoever is in it.
    ctx.at(&ren, offset(here, 90.0, 20.0)).await;
    assert_eq!(ctx.seen_by(&aoi, &ren).await.expect("a pin").temperature_celsius, Some(14.3));
    let asked = open_meteo.requests("/v1/forecast");
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].query["current"], "temperature_2m,weather_code,is_day");
    assert_eq!(
        (asked[0].query["latitude"].clone(), asked[0].query["longitude"].clone()),
        (format!("{:.2}", here.0), format!("{:.2}", here.1))
    );
}

#[tokio::test]
async fn when_the_services_are_down_the_bundled_data_answers() {
    let services = StandIn::start().await;
    services.set_down(true);
    let ctx = start_with(GeoConfig {
        geocoder_url: services.url.clone(),
        routing_url: services.url.clone(),
        weather_url: services.url.clone(),
        ..Default::default()
    })
    .await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;

    // Search and reverse geocoding fall back to the gazetteer.
    let found = search(&ctx, &aoi, "渋谷", SHIBUYA).await;
    assert!(
        found.iter().any(|result| result.name == "渋谷区"),
        "{:?}",
        found.iter().map(|r| &r.name).collect::<Vec<_>>()
    );
    let shibuya = reverse(&ctx, &aoi, SHIBUYA, "").await;
    assert_eq!(
        (shibuya.name.as_str(), shibuya.region.as_str(), shibuya.time_zone.as_str()),
        ("渋谷区", "東京都", "Asia/Tokyo")
    );
    assert_eq!(
        (services.requests("/api").len(), services.requests("/reverse").len()),
        (1, 1),
        "the geocoder was asked first"
    );

    // Directions fall back to the estimate.
    let walk = directions(&ctx, &aoi, TravelMode::TRAVEL_MODE_WALK).await;
    assert_eq!(walk.provider, "estimate");
    assert!((6300.0..6500.0).contains(&walk.distance_meters), "{}", walk.distance_meters);
    assert_eq!(services.requests("/route").len(), 1);

    // Pins simply carry no weather, and a failing service is not asked again for a while.
    ctx.at(&aoi, somewhere()).await;
    for _ in 0..50 {
        if !services.requests("/v1/forecast").is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for _ in 0..5 {
        let pin = ctx.seen_by(&ren, &aoi).await.expect("a pin");
        assert_eq!((pin.temperature_celsius, pin.weather_icon.as_str()), (None, ""));
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(services.requests("/v1/forecast").len(), 1);

    // Back up: the geocoder is used again at once.
    services.set_down(false);
    assert_eq!(search(&ctx, &aoi, "モカ", SHIBUYA).await[0].name, "モカ珈琲店");
}
