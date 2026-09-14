//! Bang / tỉnh theo quốc gia — cố định, không cần API. Port từ `Regions.swift`.

pub fn states(country: &str) -> &'static [&'static str] {
    match country {
        "US" => &[
            "Alabama", "Alaska", "Arizona", "Arkansas", "California", "Colorado",
            "Connecticut", "Delaware", "Florida", "Georgia", "Hawaii", "Idaho", "Illinois",
            "Indiana", "Iowa", "Kansas", "Kentucky", "Louisiana", "Maine", "Maryland",
            "Massachusetts", "Michigan", "Minnesota", "Mississippi", "Missouri", "Montana",
            "Nebraska", "Nevada", "New Hampshire", "New Jersey", "New Mexico", "New York",
            "North Carolina", "North Dakota", "Ohio", "Oklahoma", "Oregon", "Pennsylvania",
            "Rhode Island", "South Carolina", "South Dakota", "Tennessee", "Texas", "Utah",
            "Vermont", "Virginia", "Washington", "West Virginia", "Wisconsin", "Wyoming",
        ],
        "CA" => &[
            "Alberta", "British Columbia", "Manitoba", "New Brunswick",
            "Newfoundland and Labrador", "Northwest Territories", "Nova Scotia", "Nunavut",
            "Ontario", "Prince Edward Island", "Quebec", "Saskatchewan", "Yukon",
        ],
        "AU" => &[
            "Australian Capital Territory", "New South Wales", "Northern Territory",
            "Queensland", "South Australia", "Tasmania", "Victoria", "Western Australia",
        ],
        _ => &[],
    }
}
