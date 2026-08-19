#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "$0")/.." && pwd)
java=/home/melaton/.local/share/FjordLauncher/java/java-runtime-delta/bin/java
injector="$project_root/build/injection/Vape421LinuxInjector.jar"
force_injector="$project_root/build/injection/Vape421LinuxNativeInjector"
agent="$project_root/build/injection/libVape421Native.so"
payload="$project_root/build/injection/Vape421Payload.jar"

if [[ -t 1 && -z ${NO_COLOR:-} ]]; then
    readonly reset=$'\033[0m'
    readonly bold=$'\033[1m'
    readonly dim=$'\033[2m'
    readonly cyan=$'\033[36m'
    readonly green=$'\033[32m'
    readonly yellow=$'\033[33m'
    readonly red=$'\033[31m'
else
    readonly reset='' bold='' dim='' cyan='' green='' yellow='' red=''
fi

print_banner() {
    printf '\n%s%s' "$bold" "$cyan"
    cat <<'EOF'
  ___                   __     __
 / _ \ _ __   ___ _ __ \ \   / /_ _ _ __   ___
| | | | '_ \ / _ \ '_ \ \ \ / / _` | '_ \ / _ \
| |_| | |_) |  __/ | | | \ V / (_| | |_) |  __/
 \___/| .__/ \___|_| |_|  \_/ \__,_| .__/ \___|
      |_|                           |_|
EOF
    printf '%s%sLinux Injector%s\n' "$reset" "$dim" "$reset"
    printf '%s%s%s\n\n' "$cyan" '──────────────────────────────────────────────────────' "$reset"
}

info() {
    printf '%s%s•%s %s\n' "$bold" "$cyan" "$reset" "$*"
}

success() {
    printf '%s%s✓%s %s\n' "$bold" "$green" "$reset" "$*"
}

warn() {
    printf '%s%s!%s %s\n' "$bold" "$yellow" "$reset" "$*" >&2
}

die() {
    printf '%s%s×%s %s\n' "$bold" "$red" "$reset" "$*" >&2
    exit 1
}

usage() {
    printf '%sUsage:%s %s [--force] [minecraft-java-pid]\n' "$bold" "$reset" "$0" >&2
}

force=false
if (($# > 0)) && [[ $1 == --force ]]; then
    force=true
    shift
fi
target_owner_uid=$(id -u)
if [[ $force == true && $EUID -eq 0 && ${SUDO_UID:-} =~ ^[0-9]+$ ]]; then
    target_owner_uid=$SUDO_UID
fi

find_minecraft_pids() {
    local process_dir process_pid process_uid process_exe command_line command_line_lower

    for process_dir in /proc/[0-9]*; do
        process_pid=${process_dir##*/}
        [[ -r $process_dir/status && -r $process_dir/cmdline ]] || continue

        process_uid=$(awk '/^Uid:/ { print $2; exit }' "$process_dir/status" 2>/dev/null) || continue
        [[ $process_uid == "$target_owner_uid" ]] || continue

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
            || $command_line_lower == *org.prismlauncher.entrypoint* \
            || $command_line_lower == *newlaunch.jar* \
            || $command_line_lower == *minecraft-*client.jar* \
            || $command_line_lower == *--gamedir* \
            || $command_line_lower == *--assetsdir* ]]; then
            # Launcher command lines may contain access tokens. Do not echo them.
            printf '%s\t%s\n' "$process_pid" "$process_exe"
        fi
    done
}

choose_target_pid() {
    local candidate_pid candidate_exe choice choice_number index
    local -a candidate_pids=() candidate_exes=()

    while IFS=$'\t' read -r candidate_pid candidate_exe; do
        candidate_pids+=("$candidate_pid")
        candidate_exes+=("$candidate_exe")
    done < <(find_minecraft_pids)

    if ((${#candidate_pids[@]})); then
        success "Found ${#candidate_pids[@]} Minecraft Java process(es)"
        echo
        for index in "${!candidate_pids[@]}"; do
            printf '  %s%s[%d]%s  PID %-7s %s%s%s\n' \
                "$bold" "$cyan" "$((index + 1))" "$reset" \
                "${candidate_pids[$index]}" "$dim" "${candidate_exes[$index]}" "$reset"
        done
        echo
        printf '%sSelect a process%s [1-%d], or enter a PID: ' \
            "$bold" "$reset" "${#candidate_pids[@]}"
        read -r choice
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
        warn "No Minecraft Java process was found automatically"
        printf '%sEnter the Minecraft Java PID:%s ' "$bold" "$reset"
        read -r target_pid
    fi
}

print_banner

if (($# > 1)); then
    usage
    exit 2
fi

if (($# == 1)); then
    target_pid=$1
else
    if [[ ! -t 0 ]]; then
        warn "No interactive terminal is available; pass the Minecraft Java PID as an argument"
        usage
        exit 2
    fi
    choose_target_pid
fi

if [[ ! $target_pid =~ ^[0-9]+$ || ${#target_pid} -gt 10 ]]; then
    warn "Invalid target PID: $target_pid"
    usage
    exit 2
fi
target_pid=$((10#$target_pid))
if ((target_pid <= 1)); then
    warn "Invalid target PID: $target_pid"
    usage
    exit 2
fi

if [[ ! -r /proc/$target_pid/status ]]; then
    die "Target PID does not exist or is not readable: $target_pid"
fi
target_uid=$(awk '/^Uid:/ { print $2; exit }' "/proc/$target_pid/status")
if [[ $target_uid != "$target_owner_uid" ]]; then
    die "Refusing a JVM owned by another user (UID $target_uid)"
fi
target_exe=$(readlink -f "/proc/$target_pid/exe")
if [[ ${target_exe##*/} != java ]]; then
    die "Refusing non-Java target: $target_exe"
fi

success "Selected Minecraft JVM"
printf '  %sPID%s         %s\n' "$dim" "$reset" "$target_pid"
printf '  %sExecutable%s  %s\n\n' "$dim" "$reset" "$target_exe"

if [[ $force == false ]]; then
    while IFS= read -r -d '' target_argument; do
        if [[ $target_argument == -XX:+DisableAttachMechanism ]]; then
            warn "Target JVM $target_pid has the Attach mechanism disabled"
            cat >&2 <<EOF

The JVM was started with -XX:+DisableAttachMechanism.
HotSpot Attach cannot be enabled after that JVM has started.

For Lunar Client, set settings.enableAttach to true in:
  $HOME/.lunarclient/settings/launcher.json

Then fully restart the Minecraft game, or rerun this script with --force.
EOF
            exit 1
        fi
    done < "/proc/$target_pid/cmdline"
fi

for required in "$agent" "$payload"; do
    if [[ ! -f $required || ! -r $required ]]; then
        die "Required injection file is missing or unreadable: $required"
    fi
done

if [[ $force == true ]]; then
    if [[ ! -x $force_injector ]]; then
        die "Required native injector is missing or not executable: $force_injector"
    fi
    agent_hash=$(sha256sum "$agent" | awk '{ print substr($1, 1, 16) }')
    target_gid=$(awk '/^Gid:/ { print $2; exit }' "/proc/$target_pid/status")
    target_home=$(getent passwd "$target_owner_uid" | awk -F: '{ print $6; exit }')
    if [[ -z $target_home || ! -d $target_home ]]; then
        die "Could not resolve the target user's home directory"
    fi
    runtime_dir="$target_home/.local/state/vape4linux/injection"
    runtime_agent="$runtime_dir/libVape421Native-$agent_hash.so"
    if [[ $EUID -eq 0 ]]; then
        install -d -m 700 -o "$target_owner_uid" -g "$target_gid" -- "$runtime_dir"
        if [[ ! -f $runtime_agent ]]; then
            install -m 400 -o "$target_owner_uid" -g "$target_gid" -- \
                "$agent" "$runtime_agent"
        fi
    else
        mkdir -p "$runtime_dir"
        chmod 700 "$runtime_dir"
        if [[ ! -f $runtime_agent ]]; then
            cp -p -- "$agent" "$runtime_agent"
            chmod 400 "$runtime_agent"
        fi
    fi
    if ! cmp -s -- "$agent" "$runtime_agent"; then
        die "Immutable runtime agent does not match the build artifact: $runtime_agent"
    fi
    warn "Force mode uses ptrace and executes native code inside PID $target_pid"
    info "Bootstrap log: $runtime_dir/vape421-native-$target_pid.log"
    info "Starting native injection…"
    exec "$force_injector" "$target_pid" "$runtime_agent" "$payload"
fi

for required in "$java" "$injector"; do
    if [[ ! -f $required || ! -r $required ]]; then
        die "Required injection file is missing or unreadable: $required"
    fi
done

warn "The agent and payload inherit all permissions of PID $target_pid"
info "Starting HotSpot Attach injection…"

exec "$java" --add-modules jdk.attach -jar "$injector" \
    "$target_pid" "$agent" "$payload"
