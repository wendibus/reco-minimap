#!/usr/bin/env python3
"""Zeigt, wie viel vom Modell wirklich auf CoreML laeuft und welche Recheneinheit am schnellsten ist.

  ~/ortvenv/bin/python scripts/coreml_probe.py <modellordner|datei.onnx>

Liest nur das Modell (keine Bilder). Ausgabe: je Recheneinheit die Zahl der CoreML-Partitionen,
den Anteil der Knoten auf CoreML und die mittlere Zeit pro 512x512-Kachel.
"""
import json, os, re, subprocess, sys, tempfile, textwrap

def model_path(arg):
    if os.path.isdir(arg):
        rt = json.load(open(os.path.join(arg, "runtime.json")))
        return os.path.join(arg, rt["artifact"]["file"])
    return arg

CHILD = r'''
import sys, time, numpy as np, onnxruntime as ort
path, units = sys.argv[1], sys.argv[2]
so = ort.SessionOptions()
so.log_severity_level = 0          # verbose: ORT meldet Partitionen / Knoten
opts = {"ModelFormat": "MLProgram", "MLComputeUnits": units, "RequireStaticInputShapes": "1"}
try:
    sess = ort.InferenceSession(path, so, providers=[("CoreMLExecutionProvider", opts), "CPUExecutionProvider"])
except Exception as e:
    print("PROBE_ERR", e); sys.exit(0)
print("PROBE_PROVIDERS", sess.get_providers())
x = np.random.rand(1, 3, 512, 512).astype(np.float32)
name = sess.get_inputs()[0].name
for _ in range(3): sess.run(None, {name: x})           # Aufwaermen / Kompilieren
ts = []
for _ in range(15):
    t = time.perf_counter(); sess.run(None, {name: x}); ts.append((time.perf_counter() - t) * 1000)
ts.sort()
print("PROBE_MS", round(ts[len(ts)//2], 1), round(ts[0], 1))
'''

def main():
    if len(sys.argv) < 2:
        print(__doc__); sys.exit(1)
    path = model_path(sys.argv[1])
    print("Modell:", path)
    print(f"{'Einheiten':<20}{'Partitionen':>12}{'Knoten auf CoreML':>22}{'ms/Kachel (Median)':>22}")
    for units in ["ALL", "CPUAndNeuralEngine", "CPUAndGPU", "CPUOnly"]:
        r = subprocess.run([sys.executable, "-c", CHILD, path, units], capture_output=True, text=True)
        out = r.stdout + "\n" + r.stderr
        if "PROBE_ERR" in out:
            print(f"{units:<20}  Fehler: {re.search(r'PROBE_ERR (.*)', out).group(1)[:90]}"); continue
        pv = re.search(r"PROBE_PROVIDERS (.*)", out)
        if pv and "CoreMLExecutionProvider" not in pv.group(1):
            print(f"{units:<20}  CoreML NICHT aktiv (Provider: {pv.group(1)[:60]}) - falsche onnxruntime?"); continue
        m = re.search(r"number of partitions supported by CoreML: (\d+).*?number of nodes in the graph: (\d+).*?number of nodes supported by CoreML: (\d+)", out, re.S)
        part = nodes = "?"
        if m:
            part, total, sup = m.group(1), int(m.group(2)), int(m.group(3))
            nodes = f"{sup}/{total} ({100*sup/max(total,1):.0f}%)"
        t = re.search(r"PROBE_MS ([\d.]+)", out)
        print(f"{units:<20}{part:>12}{nodes:>22}{(t.group(1) + ' ms') if t else 'n/a':>22}")
    print(textwrap.dedent("""
    Lesehilfe: Wenige Partitionen und ein hoher Knoten-Anteil = das Modell laeuft am Stueck auf CoreML.
    Viele Partitionen = haeufiges Hin-und-Her zwischen CoreML und CPU (bremst). Die schnellste Zeile
    gilt fuer `reco-minimap ... --units <all|ane|gpu|cpu>`."""))

main()
