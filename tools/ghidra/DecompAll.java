// Headless Ghidra script: decompile every function of the program to one C-like text file.
// usage: analyzeHeadless <proj> <name> -import <bin> -postScript DecompAll.java <outfile> -deleteProject
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.listing.*;
import java.io.*;

public class DecompAll extends GhidraScript {
    @Override
    public void run() throws Exception {
        String out = getScriptArgs()[0];
        DecompInterface di = new DecompInterface();
        di.openProgram(currentProgram);
        int n = 0, bad = 0;
        try (PrintWriter w = new PrintWriter(new BufferedWriter(new FileWriter(out)))) {
            w.println("// decompiled by Ghidra " + ghidra.framework.Application.getApplicationVersion());
            w.println("// program: " + currentProgram.getName() + "  sha256: " + currentProgram.getExecutableSHA256());
            for (Function f : currentProgram.getFunctionManager().getFunctions(true)) {
                if (f.isExternal() || f.isThunk()) continue;
                DecompileResults r = di.decompileFunction(f, 60, monitor);
                w.println("\n// ---- " + f.getName() + " @ " + f.getEntryPoint() + " ----");
                if (r != null && r.decompileCompleted()) { w.println(r.getDecompiledFunction().getC()); n++; }
                else { w.println("// decompilation failed: " + (r == null ? "null" : r.getErrorMessage())); bad++; }
            }
        }
        println("decompiled " + n + " functions, " + bad + " failed");
    }
}
