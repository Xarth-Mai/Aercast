#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "usage: $0 baseline|active|stopped ALLOWED_NODE EXCLUDED_NODE [BASELINE_FILE]" >&2
    exit 2
}

[[ $# -ge 3 ]] || usage
mode=$1
allowed=$2
excluded=$3
baseline=${4:-}
graph=$(pw-dump)

case $mode in
baseline)
    [[ -n $baseline ]] || usage
    jq -e --arg allowed "$allowed" --arg excluded "$excluded" '
      . as $g
      | [.[] | select(.type == "PipeWire:Interface:Node" and
          (.info.props["node.name"] == $allowed or .info.props["node.name"] == $excluded))] as $nodes
      | if ($nodes | length) != 2 then error("expected one allowed and one excluded node") else . end
      | [.[] | select(.type == "PipeWire:Interface:Node" and .info.props["media.class"] == "Audio/Sink") | .id] as $sinks
      | [.[] | select(.type == "PipeWire:Interface:Link" and .info.state == "active")
          | select((.info.props["link.output.node"] as $output | [ $nodes[].id ] | index($output)) and
                   (.info.props["link.input.node"] as $input | $sinks | index($input)))
          | {id, serial:.info.props["object.serial"], output_node:.info.props["link.output.node"],
             output_port:.info.props["link.output.port"], input_node:.info.props["link.input.node"],
             input_port:.info.props["link.input.port"]}]
      | if length != 4 then error("expected four active stereo sink links") else sort_by(.serial) end
    ' <<<"$graph" >"$baseline"
    ;;
active)
    jq -e --arg allowed "$allowed" --arg excluded "$excluded" '
      . as $g
      | [.[] | select(.type == "PipeWire:Interface:Node" and .info.props["node.name"] == $allowed)] as $allowed_nodes
      | [.[] | select(.type == "PipeWire:Interface:Node" and .info.props["node.name"] == $excluded)] as $excluded_nodes
      | [.[] | select(.type == "PipeWire:Interface:Node" and .info.props["node.name"] == "aercast-selective-audio")] as $capture_nodes
      | if ($allowed_nodes | length) != 1 or ($excluded_nodes | length) != 1 or ($capture_nodes | length) != 1
        then error("expected exactly one allowed, excluded, and Aercast node") else . end
      | $allowed_nodes[0].id as $allowed_id
      | $excluded_nodes[0].id as $excluded_id
      | $capture_nodes[0].id as $capture_id
      | if $capture_nodes[0].info.props["node.passive"] != "in"
        then error("Aercast node.passive is not in") else . end
      | [.[] | select(.type == "PipeWire:Interface:Node" and .info.props["media.class"] == "Audio/Sink") | .id] as $sinks
      | [.[] | select(.type == "PipeWire:Interface:Port" and .info.props["node.id"] == $allowed_id and
          .info.props["port.direction"] == "out" and
          (.info.props["audio.channel"] == "FL" or .info.props["audio.channel"] == "FR"))] as $allowed_ports
      | [.[] | select(.type == "PipeWire:Interface:Link" and
          .info.props["link.output.node"] == $allowed_id and .info.props["link.input.node"] == $capture_id)] as $capture_links
      | if ($allowed_ports | length) != 2 or ($capture_links | length) != 2
        then error("allowed stereo ports do not have exactly two Aercast links") else . end
      | if any($capture_links[]; .info.state != "active")
        then error("an Aercast link is not active") else . end
      | if any($capture_links[]; .info.props["link.passive"] != null)
        then error("Aercast supplied link.passive instead of using node.passive") else . end
      | if ([$allowed_ports[] as $port
          | any($g[]; .type == "PipeWire:Interface:Link" and .info.state == "active" and
              .info.props["link.output.node"] == $allowed_id and
              .info.props["link.output.port"] == $port.id and
              (.info.props["link.input.node"] as $input | $sinks | index($input)))] | all) | not
        then error("an allowed output port lost its active sink route") else . end
      | if any(.[]; .type == "PipeWire:Interface:Link" and
          .info.props["link.output.node"] == $excluded_id and .info.props["link.input.node"] == $capture_id)
        then error("excluded Communication node entered Aercast") else . end
      | {allowed:$allowed_id, excluded:$excluded_id, capture:$capture_id,
         capture_links:[$capture_links[] | {id, serial:.info.props["object.serial"]}]}
    ' <<<"$graph"
    if [[ -n $baseline ]]; then
        jq -e --slurpfile expected "$baseline" '
          [.[] | select(.type == "PipeWire:Interface:Link")
            | {id, serial:.info.props["object.serial"], output_node:.info.props["link.output.node"],
               output_port:.info.props["link.output.port"], input_node:.info.props["link.input.node"],
               input_port:.info.props["link.input.port"]}] as $current
          | all($expected[0][]; . as $link | $current | index($link))
          | if . then true else error("a pre-existing sink link changed or disappeared") end
        ' <<<"$graph" >/dev/null
    fi
    ;;
stopped)
    jq -e --arg allowed "$allowed" --arg excluded "$excluded" '
      . as $g
      | [.[] | select(.type == "PipeWire:Interface:Node" and .info.props["node.name"] == $allowed)] as $allowed_nodes
      | [.[] | select(.type == "PipeWire:Interface:Node" and .info.props["node.name"] == $excluded)] as $excluded_nodes
      | [.[] | select(.type == "PipeWire:Interface:Node" and .info.props["node.name"] == "aercast-selective-audio")] as $capture_nodes
      | if ($excluded_nodes | length) != 1 or ($capture_nodes | length) != 1
        then error("expected one excluded and one Aercast node") else . end
      | $capture_nodes[0].id as $capture_id
      | if ($allowed_nodes | length) == 1 and $allowed_nodes[0].info.state == "running"
        then error("allowed node is still runnable") else . end
      | if any(.[]; .type == "PipeWire:Interface:Link" and .info.props["link.input.node"] == $capture_id)
        then error("Aercast still has an input link after allowed playback stopped") else . end
      | if any(.[]; .type == "PipeWire:Interface:Link" and
          .info.props["link.output.node"] == $excluded_nodes[0].id and
          .info.props["link.input.node"] == $capture_id)
        then error("excluded Communication node entered Aercast") else . end
      | {allowed_present:($allowed_nodes | length), capture_state:$capture_nodes[0].info.state}
    ' <<<"$graph"
    ;;
*) usage ;;
esac
