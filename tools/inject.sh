#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "$0")/.." && pwd)
java=/home/melaton/.local/share/FjordLauncher/java/java-runtime-delta/bin/java
injector="$project_root/build/injection/Vape421LinuxInjector.jar"
agent="$project_root/build/injection/libVape421Native.so"
payload="$project_root/build/injection/Vape421Payload.jar"

usage() {
    echo "Usage: $0 [minecraft-java-pid]" >&2
}

find_minecraft_pids() {
    local process_dir process_pid process_uid process_exe command_line command_line_lower

    for process_dir in /proc/[0-9]*; do
        process_pid=${process_dir##*/}
        [[ -r $process_dir/status && -r $process_dir/cmdline ]] || continue

        process_uid=$(awk '/^Uid:/ { print $2; exit }' "$process_dir/status" 2>/dev/null) || continue
        [[ $process_uid == "$(id -u)" ]] || continue

        process_exe=$(readlink -f "$process_dir/exe" 2>/dev/null) || continue
        [[ ${process_exe##*/} == java ]] || continue

        command_line=$(tr '\0' ' ' < "$process_dir/cmdline" 2>/dev/null) || continue
        command_line_lower=${command_line,,}
        if [[ $command_line_lower == *net.minecraft.client* \
            || $command_line_lower == *knotclient* \
            || $command_line_lower == *clientlaunchhandler* \
            || $command_line_lower == *forgeclient* \
            || $command_line_lower == *neoforgeclient* \
            || $command_line_lower == *lunarclient* \
            || $command_line_lower == *--gamedir* \
            || $command_line_lower == *--assetsdir* ]]; then
            command_line=${command_line//$'\t'/ }
            command_line=${command_line//$'\n'/ }
            printf '%s\t%s\n' "$process_pid" "$command_line"
        fi
    done
}

choose_target_pid() {
    local candidate_pid candidate_command choice choice_number index display_command
    local -a candidate_pids=() candidate_commands=()

    while IFS=$'\t' read -r candidate_pid candidate_command; do
        candidate_pids+=("$candidate_pid")
        candidate_commands+=("$candidate_command")
    done < <(find_minecraft_pids)

    if ((${#candidate_pids[@]})); then
        echo "Found Minecraft Java processes:"
        for index in "${!candidate_pids[@]}"; do
            display_command=${candidate_commands[$index]}
            if ((${#display_command} > 140)); then
                display_command="${display_command:0:137}..."
            fi
            printf '  [%d] PID %s  %s\n' \
                "$((index + 1))" "${candidate_pids[$index]}" "$display_command"
        done
        echo
        read -r -p "Select a process [1-${#candidate_pids[@]}], or enter a PID manually: " choice
        if [[ $choice =~ ^[0-9]+$ && ${#choice} -le 10 ]]; then
            choice_number=$((10#$choice))
        fi
        if [[ -n ${choice_number:-} ]] \
            && ((choice_number >= 1 && choice_number <= ${#candidate_pids[@]})); then
            target_pid=${candidate_pids[$((choice_number - 1))]}
        else
            target_pid=$choice
        fi
    else
        echo "No Minecraft Java process was found automatically."
        read -r -p "Enter the Minecraft Java PID manually: " target_pid
    fi
}

if (($# > 1)); then
    usage
    exit 2
fi

if (($# == 1)); then
    target_pid=$1
else
    if [[ ! -t 0 ]]; then
        echo "No interactive terminal is available; pass the Minecraft Java PID as an argument." >&2
        usage
        exit 2
    fi
    choose_target_pid
fi

if [[ ! $target_pid =~ ^[0-9]+$ || ${#target_pid} -gt 10 ]]; then
    echo "Invalid target PID: $target_pid" >&2
    usage
    exit 2
fi
target_pid=$((10#$target_pid))
if ((target_pid <= 1)); then
    echo "Invalid target PID: $target_pid" >&2
    usage
    exit 2
fi

if [[ ! -r /proc/$target_pid/status ]]; then
    echo "Target PID does not exist or is not readable: $target_pid" >&2
    exit 1
fi
target_uid=$(awk '/^Uid:/ { print $2; exit }' "/proc/$target_pid/status")
if [[ $target_uid != $(id -u) ]]; then
    echo "Refusing a JVM owned by another user (UID $target_uid)" >&2
    exit 1
fi
target_exe=$(readlink -f "/proc/$target_pid/exe")
if [[ ${target_exe##*/} != java ]]; then
    echo "Refusing non-Java target: $target_exe" >&2
    exit 1
fi

for required in "$java" "$injector" "$agent" "$payload"; do
    if [[ ! -f $required || ! -r $required ]]; then
        echo "Required injection file is missing or unreadable: $required" >&2
        exit 1
    fi
done

echo "WARNING: this target is outside the sandbox."
echo "The injected native agent and payload will inherit all permissions of PID $target_pid."
echo "Target executable: $target_exe"

exec "$java" --add-modules jdk.attach -jar "$injector" \
    "$target_pid" "$agent" "$payload"
