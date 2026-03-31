/**
 * Weather function that gets the current weather for a given city using the Open-Meteo API.
 * The function first retrieves the coordinates of the city using the Open-Meteo Geocoding API, and then uses those coordinates to fetch the current weather data.
 * The function expects an input object with a 'city' property, which is the name of the city for which to retrieve the weather.
 * Example input: { city: "New York" }
 * Example output: { temperature: 22.5, windspeed: 5.2, ... }
 * @param {*} input 
 * @returns 
 */
async function show(input) { 
  const { city } = input;
  const cord = await getCoordinates(city)
  return getWeather(cord);
}
 
async function getWeather(cord) {  
  const url = `https://api.open-meteo.com/v1/forecast?latitude=${cord.latitude}&longitude=${cord.longitude}&current_weather=true`;
  const response = await fetch(url);
  const data = await response.json();
  return data.current_weather;
}

async function getCoordinates(city) {
  // Use 'count=1' to get the single most relevant result
  const url = `https://geocoding-api.open-meteo.com/v1/search?name=${encodeURIComponent(city)}&count=1&language=en&format=json`;
  const response = await fetch(url);

  const data = await response.json();
  const result = data.results[0];
  // Check if any location was found
  if (!data.results) throw new Error("Location not found");

  return {
    latitude: result.latitude,
    longitude: result.longitude,
    name: result.name,
    country: result.country
  };
}

