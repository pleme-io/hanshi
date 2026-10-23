def rules()
  ["bundle_loaded", "ink_low"]
end

def bundle_loaded()
  emit_advisory(
    :info,
    "Rule bundle loaded",
    "the advisory rule bundle is live",
    "No action required.",
    "bundle-loaded"
  )
end

def ink_low()
  if json_get(json_parse(inputs_json), "ink_low")
    emit_advisory(
      :warn,
      "Ink low",
      "a cartridge reports low ink",
      "Replace the cartridge before the next job.",
      "ink-low"
    )
  end
end
